//! App framework (docs/steps/apps.md). Every app is a tab of the Apps window
//! (its own native window in the separate layout) with three labelled areas:
//! INPUTS | main view | OUTPUT. One function builds the app's script: the app
//! runs that text privately (followed by read-only probe lines that fetch what
//! its view draws), Export runs the same text in the workspace and Generate
//! script saves it under a header. The UI only calls engine builtins.
use crate::theme::ThemedScrollArea;
use crate::toolkits::{Action, AppStatus};
use eframe::egui;
use matrust::{Array, engine::app_launchers::Launch};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[path = "app_designer.rs"]
pub(crate) mod app_designer;
#[path = "basic_fitting.rs"]
pub(crate) mod basic_fitting;
#[path = "ber_analysis.rs"]
pub(crate) mod ber_analysis;
#[path = "camera_calibrator.rs"]
pub(crate) mod camera_calibrator;
#[path = "classification_learner.rs"]
pub(crate) mod classification_learner;
#[path = "color_thresholder.rs"]
pub(crate) mod color_thresholder;
#[path = "comparison_tool.rs"]
pub(crate) mod comparison_tool;
#[path = "control_system_designer.rs"]
pub(crate) mod control_system_designer;
#[path = "curve_fitter.rs"]
pub(crate) mod curve_fitter;
#[path = "data_cleaner.rs"]
pub(crate) mod data_cleaner;
#[path = "data_statistics.rs"]
pub(crate) mod data_statistics;
#[path = "dicom_browser.rs"]
pub(crate) mod dicom_browser;
#[path = "distribution_explorer.rs"]
pub(crate) mod distribution_explorer;
#[path = "distribution_fitter.rs"]
pub(crate) mod distribution_fitter;
#[path = "econometric_modeler.rs"]
pub(crate) mod econometric_modeler;
#[path = "filter_visualizer.rs"]
pub(crate) mod filter_visualizer;
#[path = "fuzzy_designer.rs"]
pub(crate) mod fuzzy_designer;
#[path = "image_batch_processor.rs"]
pub(crate) mod image_batch_processor;
#[path = "image_region_analyzer.rs"]
pub(crate) mod image_region_analyzer;
#[path = "image_segmenter.rs"]
pub(crate) mod image_segmenter;
#[path = "image_viewer.rs"]
pub(crate) mod image_viewer;
#[path = "import_tool.rs"]
pub(crate) mod import_tool;
#[path = "linear_system_analyzer.rs"]
pub(crate) mod linear_system_analyzer;
#[path = "model_reducer.rs"]
pub(crate) mod model_reducer;
#[path = "mpc_designer.rs"]
pub(crate) mod mpc_designer;
#[path = "multires_analyzer.rs"]
pub(crate) mod multires_analyzer;
#[path = "neural_net_apps.rs"]
pub(crate) mod neural_net_apps;
#[path = "optimize_app.rs"]
pub(crate) mod optimize_app;
#[path = "pde_modeler.rs"]
pub(crate) mod pde_modeler;
#[path = "pid_tuner.rs"]
pub(crate) mod pid_tuner;
#[path = "profiler_app.rs"]
pub(crate) mod profiler_app;
#[path = "registration_estimator.rs"]
pub(crate) mod registration_estimator;
#[path = "regression_learner.rs"]
pub(crate) mod regression_learner;
#[path = "signal_analyzer.rs"]
pub(crate) mod signal_analyzer;
#[path = "solve_ode_app.rs"]
pub(crate) mod solve_ode_app;
#[path = "spline_tool.rs"]
pub(crate) mod spline_tool;
#[path = "system_identification.rs"]
pub(crate) mod system_identification;
pub(crate) mod video_viewer;
#[path = "volume_viewer.rs"]
pub(crate) mod volume_viewer;
#[path = "waveform_generator.rs"]
pub(crate) mod waveform_generator;
#[path = "wavelet_denoiser.rs"]
pub(crate) mod wavelet_denoiser;
#[path = "wavelet_tf_analyzer.rs"]
pub(crate) mod wavelet_tf_analyzer;
#[path = "window_designer.rs"]
pub(crate) mod window_designer;

/// Apply each window's final request after any figure reveal in the same job.
pub(super) fn receive_launches(
    workspace: &mut crate::Workspace,
    launches: Vec<Launch>,
    mut profiler_viewer: Option<matrust::Result<Array>>,
) {
    use matrust::session::ToolTab;
    for launch in launches {
        match launch {
            Launch::App(ToolTab::ComparisonTool) => workspace.receive_comparison_tool(),
            Launch::App(ToolTab::VideoViewer) => workspace.receive_video_viewer(),
            Launch::App(ToolTab::DicomBrowser) => dicom_browser::receive_open(workspace),
            Launch::App(ToolTab::MpcDesigner) => mpc_designer::receive_open(workspace),
            Launch::App(ToolTab::FuzzyLogicDesigner) => fuzzy_designer::receive_open(workspace),
            Launch::App(ToolTab::RegistrationEstimator) => {
                registration_estimator::receive_open(workspace)
            }
            Launch::App(ToolTab::CameraCalibrator) => camera_calibrator::receive_open(workspace),
            Launch::App(ToolTab::VolumeViewer) => volume_viewer::receive_open(workspace),
            Launch::App(ToolTab::AppDesigner) => app_designer::receive_open(workspace),
            Launch::App(ToolTab::PdeModeler) => pde_modeler::receive_open(workspace),
            Launch::App(ToolTab::Profiler) => {
                if let Some(info) = profiler_viewer.take() {
                    profiler_app::receive_viewer(workspace, info);
                }
            }
            _ if workspace.worker.cancel.load(Ordering::Relaxed) => {}
            Launch::AppsHome => {
                workspace.toolkits.deselect_tool();
                workspace.apps_home = true;
                workspace.layout.toolstrip = matrust::session::ToolstripTab::Apps;
            }
            Launch::RandomSamples | Launch::App(ToolTab::DistributionExplorer) => {
                workspace.toolkits.apps.explorer.app.mode = if launch == Launch::RandomSamples {
                    distribution_explorer::Mode::Samples
                } else {
                    distribution_explorer::Mode::Explore
                };
                workspace.open_app(crate::toolkits::Tool::App(AppKind::DistributionExplorer));
            }
            Launch::App(tab) => {
                if let Some(kind) = AppKind::from_tab(tab) {
                    workspace.open_app(crate::toolkits::Tool::App(kind));
                }
            }
        }
    }
}

/// The batch-1 apps. Only built apps reach the toolstrip, menus and the Apps
/// home; the others are listed so sessions and plans can name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppKind {
    CurveFitter,
    SignalAnalyzer,
    ImportTool,
    PidTuner,
    ControlSystemDesigner,
    LinearSystemAnalyzer,
    DistributionFitter,
    DistributionExplorer,
    DataCleaner,
    Optimize,
    SolveOde,
    BasicFitting,
    DataStatistics,
    WindowDesigner,
    SplineTool,
    ClassificationLearner,
    RegressionLearner,
    ImageSegmenter,
    ImageRegionAnalyzer,
    ColorThresholder,
    ImageViewer,
    VideoViewer,
    ImageBatchProcessor,
    ModelReducer,
    SystemIdentification,
    FilterVisualizer,
    WaveletDenoiser,
    MultiresAnalyzer,
    WaveletTfAnalyzer,
    ComparisonTool,
    Profiler,
    NeuralNetFitting,
    NeuralNetPatternRecognition,
    NeuralNetClustering,
    NeuralNetTimeSeries,
    FuzzyLogicDesigner,
    BitErrorRateAnalysis,
    WirelessWaveformGenerator,
    EconometricModeler,
    MpcDesigner,
    DicomBrowser,
    RegistrationEstimator,
    CameraCalibrator,
    VolumeViewer,
    AppDesigner,
    PdeModeler,
}
impl AppKind {
    pub const ALL: [Self; 46] = [
        Self::CurveFitter,
        Self::SignalAnalyzer,
        Self::ImportTool,
        Self::PidTuner,
        Self::ControlSystemDesigner,
        Self::LinearSystemAnalyzer,
        Self::DistributionFitter,
        Self::DistributionExplorer,
        Self::DataCleaner,
        Self::Optimize,
        Self::SolveOde,
        Self::BasicFitting,
        Self::DataStatistics,
        Self::WindowDesigner,
        Self::SplineTool,
        Self::ClassificationLearner,
        Self::RegressionLearner,
        Self::ImageSegmenter,
        Self::ImageRegionAnalyzer,
        Self::ColorThresholder,
        Self::ImageViewer,
        Self::VideoViewer,
        Self::ImageBatchProcessor,
        Self::ModelReducer,
        Self::SystemIdentification,
        Self::FilterVisualizer,
        Self::WaveletDenoiser,
        Self::MultiresAnalyzer,
        Self::WaveletTfAnalyzer,
        Self::ComparisonTool,
        Self::Profiler,
        Self::NeuralNetFitting,
        Self::NeuralNetPatternRecognition,
        Self::NeuralNetClustering,
        Self::NeuralNetTimeSeries,
        Self::FuzzyLogicDesigner,
        Self::BitErrorRateAnalysis,
        Self::WirelessWaveformGenerator,
        Self::EconometricModeler,
        Self::MpcDesigner,
        Self::DicomBrowser,
        Self::RegistrationEstimator,
        Self::CameraCalibrator,
        Self::VolumeViewer,
        Self::AppDesigner,
        Self::PdeModeler,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ComparisonTool => "Comparison Tool",
            Self::ColorThresholder => "Color Thresholder",
            Self::BasicFitting => "Basic Fitting",
            Self::CurveFitter => "Curve Fitter",
            Self::SignalAnalyzer => "Signal Analyzer",
            Self::ImportTool => "Import Tool",
            Self::PidTuner => "PID Tuner",
            Self::ControlSystemDesigner => "Control System Designer",
            Self::LinearSystemAnalyzer => "Linear System Analyzer",
            Self::DistributionFitter => "Distribution Fitter",
            Self::DistributionExplorer => "Probability Distributions",
            Self::DataCleaner => "Data Cleaner",
            Self::Optimize => "Optimize",
            Self::SolveOde => "Solve ODE",
            Self::DataStatistics => "Data Statistics",
            Self::ImageViewer => "Image Viewer",
            Self::VideoViewer => "Video Viewer",
            Self::RegressionLearner => "Regression Learner",
            Self::WindowDesigner => "Window Designer",
            Self::SplineTool => "Spline Tool",
            Self::ClassificationLearner => "Classification Learner",
            Self::ImageSegmenter => "Image Segmenter",
            Self::ImageRegionAnalyzer => "Image Region Analyzer",
            Self::ImageBatchProcessor => "Image Batch Processor",
            Self::ModelReducer => "Model Reducer",
            Self::SystemIdentification => "System Identification",
            Self::FilterVisualizer => "Filter Visualization",
            Self::WaveletDenoiser => "Wavelet Signal Denoiser",
            Self::MultiresAnalyzer => "Signal Multiresolution Analyzer",
            Self::WaveletTfAnalyzer => "Wavelet Time-Frequency Analyzer",
            Self::Profiler => "Profiler",
            Self::NeuralNetFitting => "Neural Net Fitting",
            Self::NeuralNetPatternRecognition => "Neural Net Pattern Recognition",
            Self::NeuralNetClustering => "Neural Net Clustering",
            Self::NeuralNetTimeSeries => "Neural Net Time Series",
            Self::FuzzyLogicDesigner => "Fuzzy Logic Designer",
            Self::BitErrorRateAnalysis => "Bit Error Rate Analysis",
            Self::WirelessWaveformGenerator => "Wireless Waveform Generator",
            Self::EconometricModeler => "Econometric Modeler",
            Self::MpcDesigner => "MPC Designer",
            Self::DicomBrowser => "DICOM Browser",
            Self::RegistrationEstimator => "Registration Estimator",
            Self::CameraCalibrator => "Camera Calibrator",
            Self::VolumeViewer => "Volume Viewer",
            Self::AppDesigner => "App Designer",
            Self::PdeModeler => "PDE Modeler",
        }
    }
    /// One line for the Apps home.
    pub fn detail(self) -> &'static str {
        match self {
            Self::DicomBrowser => "Scan image series, inspect frames and export DICOM volumes.",
            Self::RegistrationEstimator => {
                "Align a moving image with a fixed image and compare registration runs."
            }
            Self::VolumeViewer => {
                "Render workspace and DICOM volumes and explore orthogonal slices."
            }
            Self::AppDesigner => {
                "Build graphical MATLAB apps on a grid canvas and write component callbacks."
            }
            Self::PdeModeler => {
                "Solve 2-D elliptic, parabolic, hyperbolic and eigenvalue PDEs on drawn domains."
            }
            Self::CameraCalibrator => {
                "Calibrate camera intrinsics, extrinsics and lens distortion from checkerboard images."
            }
            Self::MpcDesigner => {
                "Configure predictive controllers, signal roles, weights and constraints."
            }
            Self::EconometricModeler => {
                "Prepare time series and retain reproducible stationarity and dependence diagnostics."
            }
            Self::BitErrorRateAnalysis => {
                "Compare theoretical and simulated error rates with reproducible datasets."
            }
            Self::WirelessWaveformGenerator => {
                "Build, impair and inspect seeded wireless waveforms."
            }
            Self::ComparisonTool => "Compare text, MAT variables and files in folders.",
            Self::Profiler => "Inspect function calls, timings and source line costs.",
            Self::FuzzyLogicDesigner => {
                "Build fuzzy systems and inspect rules, memberships and surfaces."
            }
            Self::NeuralNetFitting => "Train a shallow network to predict numeric responses.",
            Self::NeuralNetPatternRecognition => "Train a shallow network to recognize classes.",
            Self::NeuralNetClustering => "Group samples with a self-organizing map.",
            Self::NeuralNetTimeSeries => "Train delay networks and predict future responses.",
            Self::ClassificationLearner => {
                "Train and compare classifiers with reproducible validation."
            }
            Self::ColorThresholder => "Select image pixels with colour channel ranges.",
            Self::VideoViewer => "Play and inspect video files and workspace sequences.",
            Self::ImageViewer => {
                "Inspect pixels, crop images, adjust contrast and measure distances."
            }
            Self::RegressionLearner => "Compare regression models with reproducible validation.",
            Self::BasicFitting => "Fit polynomials and interpolants to plotted lines.",
            Self::CurveFitter => "Fit curves and surfaces to workspace data.",
            Self::SignalAnalyzer => "Explore signals in time and frequency.",
            Self::ImportTool => "Read text and spreadsheet files into variables.",
            Self::PidTuner => "Tune P, I, PI, PD and PID controllers for a plant model.",
            Self::ControlSystemDesigner => {
                "Shape a compensator on root locus, Bode and Nichols editors."
            }
            Self::LinearSystemAnalyzer => {
                "Compare time and frequency responses of LTI models side by side."
            }
            Self::DistributionFitter => "Fit probability distributions to data and compare them.",
            Self::DistributionExplorer => "Explore pdf and cdf shapes and draw random samples.",
            Self::DataStatistics => "Summarize plotted lines and workspace vectors.",
            Self::SplineTool => "Construct, smooth and compare splines from workspace data.",
            Self::WindowDesigner => {
                "Compare spectral windows and measure their frequency responses."
            }
            Self::DataCleaner => {
                "Fill or remove missing values and outliers, smooth, normalize and retime tables."
            }
            Self::Optimize => {
                "Minimize functions, solve equations and fit models with the solvers."
            }
            Self::SolveOde => "Solve initial value problems y' = f(t, y) and plot the solution.",
            Self::ImageSegmenter => {
                "Build a binary mask of an image from threshold, region and morphology steps."
            }
            Self::ImageRegionAnalyzer => {
                "Measure, sort and filter the connected regions of a binary image."
            }
            Self::ImageBatchProcessor => {
                "Run one image function over every file of a folder and collect the results."
            }
            Self::ModelReducer => {
                "Reduce the order of an LTI model and compare it with the original."
            }
            Self::SystemIdentification => {
                "Estimate and validate dynamic models from measured input and output data."
            }
            Self::FilterVisualizer => {
                "Compare magnitude, phase, delay, time responses and pole-zero plots of filters."
            }
            Self::WaveletDenoiser => {
                "Remove noise from a signal with wavelet thresholds and compare the results."
            }
            Self::MultiresAnalyzer => {
                "Split a signal into wavelet, EMD, VMD or EWT components and rebuild it."
            }
            Self::WaveletTfAnalyzer => {
                "Scalograms with cwt or wsst, and region reconstruction with the inverse."
            }
        }
    }
    /// Apps with a UI.
    pub fn is_built(self) -> bool {
        matches!(
            self,
            Self::BasicFitting
                | Self::CurveFitter
                | Self::SignalAnalyzer
                | Self::ImportTool
                | Self::PidTuner
                | Self::ControlSystemDesigner
                | Self::LinearSystemAnalyzer
                | Self::DistributionFitter
                | Self::DistributionExplorer
                | Self::DataCleaner
                | Self::Optimize
                | Self::SolveOde
                | Self::DataStatistics
                | Self::WindowDesigner
                | Self::SplineTool
                | Self::ClassificationLearner
                | Self::RegressionLearner
                | Self::ImageSegmenter
                | Self::ImageRegionAnalyzer
                | Self::ColorThresholder
                | Self::VideoViewer
                | Self::ImageViewer
                | Self::ImageBatchProcessor
                | Self::ModelReducer
                | Self::SystemIdentification
                | Self::FilterVisualizer
                | Self::WaveletDenoiser
                | Self::MultiresAnalyzer
                | Self::WaveletTfAnalyzer
                | Self::ComparisonTool
                | Self::Profiler
                | Self::NeuralNetFitting
                | Self::NeuralNetPatternRecognition
                | Self::NeuralNetClustering
                | Self::NeuralNetTimeSeries
                | Self::FuzzyLogicDesigner
                | Self::BitErrorRateAnalysis
                | Self::WirelessWaveformGenerator
                | Self::EconometricModeler
                | Self::MpcDesigner
                | Self::DicomBrowser
                | Self::RegistrationEstimator
                | Self::CameraCalibrator
                | Self::VolumeViewer
                | Self::AppDesigner
                | Self::PdeModeler
        )
    }
    pub fn built() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|kind| kind.is_built())
    }
    pub fn tab(self) -> matrust::session::ToolTab {
        use matrust::session::ToolTab;
        match self {
            Self::ComparisonTool => ToolTab::ComparisonTool,
            Self::ColorThresholder => ToolTab::ColorThresholder,
            Self::BasicFitting => ToolTab::BasicFitting,
            Self::CurveFitter => ToolTab::CurveFitter,
            Self::SignalAnalyzer => ToolTab::SignalAnalyzer,
            Self::ImportTool => ToolTab::ImportTool,
            Self::PidTuner => ToolTab::PidTuner,
            Self::ControlSystemDesigner => ToolTab::ControlSystemDesigner,
            Self::LinearSystemAnalyzer => ToolTab::LinearSystemAnalyzer,
            Self::DistributionFitter => ToolTab::DistributionFitter,
            Self::DistributionExplorer => ToolTab::DistributionExplorer,
            Self::DataCleaner => ToolTab::DataCleaner,
            Self::Optimize => ToolTab::Optimize,
            Self::SolveOde => ToolTab::SolveOde,
            Self::DataStatistics => ToolTab::DataStatistics,
            Self::ImageViewer => ToolTab::ImageViewer,
            Self::VideoViewer => ToolTab::VideoViewer,
            Self::RegressionLearner => ToolTab::RegressionLearner,
            Self::WindowDesigner => ToolTab::WindowDesigner,
            Self::SplineTool => ToolTab::SplineTool,
            Self::ClassificationLearner => ToolTab::ClassificationLearner,
            Self::ImageSegmenter => ToolTab::ImageSegmenter,
            Self::ImageRegionAnalyzer => ToolTab::ImageRegionAnalyzer,
            Self::ImageBatchProcessor => ToolTab::ImageBatchProcessor,
            Self::ModelReducer => ToolTab::ModelReducer,
            Self::SystemIdentification => ToolTab::SystemIdentification,
            Self::FilterVisualizer => ToolTab::FilterVisualizer,
            Self::WaveletDenoiser => ToolTab::WaveletDenoiser,
            Self::MultiresAnalyzer => ToolTab::MultiresAnalyzer,
            Self::WaveletTfAnalyzer => ToolTab::WaveletTfAnalyzer,
            Self::Profiler => ToolTab::Profiler,
            Self::NeuralNetFitting => ToolTab::NeuralNetFitting,
            Self::NeuralNetPatternRecognition => ToolTab::NeuralNetPatternRecognition,
            Self::NeuralNetClustering => ToolTab::NeuralNetClustering,
            Self::NeuralNetTimeSeries => ToolTab::NeuralNetTimeSeries,
            Self::FuzzyLogicDesigner => ToolTab::FuzzyLogicDesigner,
            Self::BitErrorRateAnalysis => ToolTab::BitErrorRateAnalysis,
            Self::WirelessWaveformGenerator => ToolTab::WirelessWaveformGenerator,
            Self::EconometricModeler => ToolTab::EconometricModeler,
            Self::MpcDesigner => ToolTab::MpcDesigner,
            Self::DicomBrowser => ToolTab::DicomBrowser,
            Self::RegistrationEstimator => ToolTab::RegistrationEstimator,
            Self::CameraCalibrator => ToolTab::CameraCalibrator,
            Self::VolumeViewer => ToolTab::VolumeViewer,
            Self::AppDesigner => ToolTab::AppDesigner,
            Self::PdeModeler => ToolTab::PdeModeler,
        }
    }
    pub fn from_tab(tab: matrust::session::ToolTab) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.tab() == tab)
    }
    /// Suggested file name of a generated script.
    #[cfg_attr(test, allow(dead_code))]
    fn script_file(self) -> &'static str {
        match self {
            Self::ComparisonTool => "comparison.m",
            Self::ColorThresholder => "createMask.m",
            Self::BasicFitting => "basic_fitting.m",
            Self::CurveFitter => "curve_fit.m",
            Self::SignalAnalyzer => "signal_analysis.m",
            Self::ImportTool => "import_data.m",
            Self::PidTuner => "pid_tuning.m",
            Self::ControlSystemDesigner => "control_design.m",
            Self::LinearSystemAnalyzer => "linear_system_analysis.m",
            Self::DistributionFitter => "distribution_fit.m",
            Self::DistributionExplorer => "probability_distributions.m",
            Self::DataCleaner => "data_cleaning.m",
            Self::Optimize => "optimization.m",
            Self::SolveOde => "solve_ode.m",
            Self::DataStatistics => "data_statistics.m",
            Self::ImageViewer => "image_viewer.m",
            Self::VideoViewer => "video_viewer.m",
            Self::RegressionLearner => "regression_learner.m",
            Self::WindowDesigner => "window_design.m",
            Self::SplineTool => "spline_tool.m",
            Self::ClassificationLearner => "classification_learner.m",
            Self::ImageSegmenter => "segment_image.m",
            Self::ImageRegionAnalyzer => "image_regions.m",
            Self::ImageBatchProcessor => "batch_process_images.m",
            Self::ModelReducer => "model_reduction.m",
            Self::SystemIdentification => "system_identification.m",
            Self::FilterVisualizer => "filter_visualization.m",
            Self::WaveletDenoiser => "wavelet_denoise.m",
            Self::MultiresAnalyzer => "multiresolution_analysis.m",
            Self::WaveletTfAnalyzer => "wavelet_time_frequency.m",
            Self::Profiler => "profile_code.m",
            Self::NeuralNetFitting => "neural_fitting.m",
            Self::NeuralNetPatternRecognition => "neural_pattern.m",
            Self::NeuralNetClustering => "neural_clustering.m",
            Self::NeuralNetTimeSeries => "neural_time_series.m",
            Self::FuzzyLogicDesigner => "fuzzy_design.m",
            Self::BitErrorRateAnalysis => "ber_analysis.m",
            Self::WirelessWaveformGenerator => "wireless_waveform.m",
            Self::EconometricModeler => "econometric_modeler.m",
            Self::MpcDesigner => "mpc_designer.m",
            Self::DicomBrowser => "dicom_browser.m",
            Self::RegistrationEstimator => "register_images.m",
            Self::CameraCalibrator => "camera_calibrator.m",
            Self::VolumeViewer => "volume_viewer.m",
            Self::AppDesigner => "app1.m",
            Self::PdeModeler => "pde_modeler.m",
        }
    }
}

// ---------------------------------------------------------------------------
// Private runs

/// Values a run leaves, by name.
pub type Outputs = BTreeMap<String, Array>;

/// One private run: the inputs become workspace variables of a fresh Engine,
/// `code` runs, and `outputs` are read back.
pub struct Request {
    pub key: u64,
    pub inputs: Vec<(String, Array)>,
    pub code: String,
    pub outputs: Vec<String>,
}

/// Runs `request` in `engine`; the error is the engine's own message.
pub fn run(engine: &mut matrust::Engine, request: Request) -> Result<Outputs, String> {
    for (name, array) in request.inputs {
        engine.insert_array(&name, array)?;
    }
    engine.execute(&request.code)?;
    request
        .outputs
        .into_iter()
        .map(|name| match engine.array(&name) {
            Some(array) => Ok((name, array)),
            None => Err(format!("The app's script did not create {name}")),
        })
        .collect()
}

/// A private script task that may also prepare a UI-independent view model.
pub(crate) type RunnerTask =
    Box<dyn FnOnce(&mut matrust::Engine) -> Result<Outputs, String> + Send>;

struct Pending {
    /// Still the job for the current request; a retired job's result,
    /// cancelled or not, is dropped even if its key is asked for again.
    current: bool,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<Result<Outputs, String>>,
    thread: std::thread::JoinHandle<()>,
}

/// Computes off the UI thread with a private Engine, like the Filter
/// Designer preview: one job at a time, a new key cancels the running job
/// and starts when it has stopped, and a finished job repaints.
#[derive(Default)]
pub struct Runner {
    key: Option<u64>,
    queued: Option<RunnerTask>,
    pending: Option<Pending>,
    result: Option<Result<Outputs, String>>,
    /// Folder the run's Engine starts in (its function files are found);
    /// None keeps the process folder.
    folder: Option<PathBuf>,
}
impl Runner {
    /// The folder later runs start in.
    pub fn set_folder(&mut self, folder: Option<&std::path::Path>) {
        if self.folder.as_deref() != folder {
            self.stop();
            self.folder = folder.map(std::path::Path::to_path_buf);
        }
    }
    /// Asks for `request`'s result. The current key keeps its run or result.
    pub fn submit(&mut self, ctx: &egui::Context, request: Request) {
        if self.key == Some(request.key) {
            self.poll(ctx);
            return;
        }
        let key = request.key;
        self.submit_task(ctx, key, Box::new(move |engine| run(engine, request)));
    }
    /// Same scheduling, cancellation and stale-result rules as script requests.
    pub(crate) fn submit_task(&mut self, ctx: &egui::Context, key: u64, task: RunnerTask) {
        if self.key != Some(key) {
            self.retire();
            self.key = Some(key);
            self.result = None;
            self.queued = Some(task);
        }
        self.poll(ctx);
    }
    /// Collects a finished job and starts the queued one.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.thread.is_finished())
        {
            let pending = self.pending.take().unwrap();
            if pending.current {
                self.result = Some(
                    pending
                        .receive
                        .try_recv()
                        .unwrap_or_else(|_| Err("The app's worker stopped".into())),
                );
            }
            let _ = pending.thread.join();
        }
        if self.pending.is_none()
            && let Some(request) = self.queued.take()
        {
            let mut engine = matrust::Engine::with_limits(matrust::engine::Limits::local());
            if let Some(folder) = &self.folder {
                // A missing folder keeps the process folder.
                let _ = engine.set_file_directory(folder);
            }
            let cancel = engine.cancellation();
            let (send, receive) = mpsc::sync_channel(1);
            let context = ctx.clone();
            let thread = std::thread::spawn(move || {
                let _ = send.send(request(&mut engine));
                context.request_repaint();
            });
            self.pending = Some(Pending {
                current: true,
                cancel,
                receive,
                thread,
            });
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    /// Cancels the running job and drops its result.
    fn retire(&mut self) {
        if let Some(pending) = &mut self.pending {
            pending.cancel.store(true, Ordering::Relaxed);
            pending.current = false;
        }
    }
    /// Cancels and forgets the current key, so asking for it again reruns
    /// instead of returning the cancelled job's error.
    pub fn stop(&mut self) {
        self.retire();
        self.key = None;
        self.queued = None;
        self.result = None;
    }
    pub fn key(&self) -> Option<u64> {
        self.key
    }
    /// The finished result for the current key.
    pub fn result(&self) -> Option<&Result<Outputs, String>> {
        self.result.as_ref()
    }
    pub fn running(&self) -> bool {
        self.pending.is_some() || self.queued.is_some()
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.retire();
        if let Some(pending) = self.pending.take() {
            let _ = pending.thread.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Workspace snapshot

/// The apps' copy of the workspace variables (shared storage, no data
/// copies). Each variable has a version that changes with its value, so a
/// run is redone only when an input it reads changed.
#[derive(Default)]
pub struct Snapshot {
    vars: BTreeMap<String, (Array, u64)>,
    next: u64,
    /// The desktop's current folder, where function files are found.
    pub folder: Option<PathBuf>,
}
impl Snapshot {
    pub fn sync(&mut self, vars: &BTreeMap<String, Array>) {
        let unchanged = self.vars.len() == vars.len()
            && self
                .vars
                .iter()
                .zip(vars)
                .all(|((a, (x, _)), (b, y))| a == b && x.same_snapshot(y));
        if unchanged {
            return;
        }
        let mut next = self.next;
        let synced = vars
            .iter()
            .map(|(name, array)| {
                let version = match self.vars.get(name) {
                    Some((old, version)) if old.same_snapshot(array) => *version,
                    _ => {
                        next += 1;
                        next
                    }
                };
                (name.clone(), (array.clone(), version))
            })
            .collect();
        self.vars = synced;
        self.next = next;
    }
    pub fn get(&self, name: &str) -> Option<&Array> {
        self.vars.get(name).map(|(array, _)| array)
    }
    pub fn version(&self, name: &str) -> Option<u64> {
        self.vars.get(name).map(|(_, version)| *version)
    }
    pub fn contains(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }
    /// Every variable, by name.
    pub fn variables(&self) -> impl Iterator<Item = (&str, &Array)> {
        self.vars
            .iter()
            .map(|(name, (array, _))| (name.as_str(), array))
    }
    /// The desktop's current folder (None in headless runs).
    pub fn folder(&self) -> Option<&std::path::Path> {
        self.folder.as_deref()
    }
    pub fn set_folder(&mut self, folder: Option<PathBuf>) {
        self.folder = folder;
    }
    /// Variables holding a tf, zpk or ss model, with that class.
    pub fn lti_models(&self) -> impl Iterator<Item = (&str, &'static str)> {
        self.vars.iter().filter_map(|(name, (array, _))| {
            matrust::engine::lti_class(array)
                .filter(|class| matches!(*class, "tf" | "zpk" | "ss"))
                .map(|class| (name.as_str(), class))
        })
    }
}

/// `base`, else `base1`, `base2`, ...: the first name `taken` refuses.
pub fn unused_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|name| !taken(name))
        .unwrap()
}

/// A number written so the engine reads back the same double.
pub fn number(v: f64) -> String {
    if v.fract() == 0. && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:?}")
    }
}

/// About five significant digits for the Output block.
pub fn short(v: f64) -> String {
    if v.is_nan() {
        return "—".into();
    }
    if v.is_infinite() {
        return if v > 0. { "Inf" } else { "-Inf" }.into();
    }
    if v == 0. {
        return "0".into();
    }
    let magnitude = v.abs().log10().floor();
    if !(-3. ..5.).contains(&magnitude) {
        return format!("{v:.4e}");
    }
    let decimals = (4. - magnitude).max(0.) as usize;
    let text = format!("{v:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Header of a saved script: which app and when (UTC).
pub fn script_header(kind: AppKind, unix_seconds: u64) -> String {
    let (year, month, day, hour, minute, _) = matrust::bench::utc_parts(unix_seconds);
    format!(
        "% {} (MatRust app), generated {year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC.\n\
         % Running this script reproduces the app's result.\n",
        kind.label()
    )
}

/// The saved script: the header, then the app's script unchanged.
pub fn saved_script(kind: AppKind, unix_seconds: u64, script: &str) -> String {
    format!("{}{script}", script_header(kind, unix_seconds))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------
// The app contract and its host

/// One app on the framework.
pub(crate) trait App {
    fn kind(&self) -> AppKind;
    /// The script that every output uses, with the user's output names; the
    /// error says what to fix in the inputs.
    fn script(&self, ws: &Snapshot) -> Result<String, String>;
    /// Prompt shown while a preview is waiting for its initial input.
    fn idle_hint(&self) -> Option<&'static str> {
        None
    }
    /// Validate controls that remain editable while waiting for the initial input.
    fn validate_idle_inputs(&self, _ws: &Snapshot) -> Result<(), String> {
        Ok(())
    }
    /// Invocation plus the same function text for apps that save a callable function.
    fn execution_script(&self, ws: &Snapshot) -> Result<String, String> {
        self.script(ws)
    }
    /// Some viewers export a small snapshot alongside their reproducing call.
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.execution_script(ws)
    }
    /// Workspace variables the script reads.
    fn inputs(&self) -> Vec<String>;
    /// Graphics setup for a figure companion in its private Engine only.
    fn preview_setup(&self) -> String {
        String::new()
    }
    /// Private lines after the script that compute what the view draws (they
    /// only read the script's variables) and the names they leave.
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>);
    /// A successful private run of the current script; an error is shown
    /// in the status line.
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String>;
    /// Apps with engine view models can prepare them on the same worker.
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, request: Request) {
        runner.submit(ctx, request);
    }
    /// Called once per frame before the script is built.
    fn prepare(&mut self, _ws: &Snapshot) {}
    /// Private lines before the script, never exported or saved (for
    /// example the private engine's current folder).
    fn prelude(&self, _ws: &Snapshot) -> String {
        String::new()
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot);
    /// Caption of the main view.
    fn main_caption(&self) -> &'static str;
    fn main_ui(&mut self, ui: &mut egui::Ui);
    /// Rows of the Output column's result block.
    fn results(&self) -> Vec<(&'static str, String)>;
    /// Editable output names, under the result block.
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot);
    /// Output names the export creates (for its message).
    fn exported(&self, ws: &Snapshot) -> Vec<String>;
    /// Status bar text of a successful run.
    fn summary(&self) -> Option<String>;
    /// Caption of the Export button.
    fn export_label(&self) -> &'static str {
        "Export to workspace"
    }
}

/// Runs an app: keeps its private run in step with the inputs, lays out the
/// three areas and turns the Output buttons into actions.
pub(crate) struct Host<A: App> {
    pub app: A,
    runner: Runner,
    delivered: Option<u64>,
    script_error: Option<String>,
    run_error: Option<String>,
    pub message: String,
    saving: bool,
}
impl<A: App> Host<A> {
    pub fn new(app: A) -> Self {
        Self {
            app,
            runner: Runner::default(),
            delivered: None,
            script_error: None,
            run_error: None,
            message: String::new(),
            saving: false,
        }
    }
    /// The private run for the current inputs: the script, then the probe.
    pub fn request(&self, ws: &Snapshot) -> Result<Request, String> {
        let script = self.app.execution_script(ws)?;
        let (probe, outputs) = self.app.probe(ws);
        let inputs = self
            .app
            .inputs()
            .into_iter()
            .map(|name| match ws.get(&name) {
                Some(array) => Ok((name, array.clone())),
                None => Err(format!("Workspace variable {name} does not exist")),
            })
            .collect::<Result<Vec<_>, String>>()?;
        // The probe runs before the script's local functions, which end it.
        let (body, functions) = split_local_functions(&script);
        let setup = self.app.preview_setup();
        let prelude = self.app.prelude(ws);
        let code = format!("{setup}{prelude}{body}{probe}{functions}");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        self.runner.folder.hash(&mut hasher);
        for (name, _) in &inputs {
            name.hash(&mut hasher);
            ws.version(name).hash(&mut hasher);
        }
        Ok(Request {
            key: hasher.finish(),
            inputs,
            code,
            outputs,
        })
    }
    /// Starts the run the inputs need and delivers a finished one.
    pub fn refresh(&mut self, ctx: &egui::Context, ws: &Snapshot) {
        self.app.prepare(ws);
        if self.app.idle_hint().is_some() {
            self.runner.stop();
            self.runner.poll(ctx);
            self.delivered = None;
            self.script_error = self.app.validate_idle_inputs(ws).err();
            self.run_error = None;
            return;
        }
        match self.request(ws) {
            Ok(request) => {
                self.script_error = None;
                self.app.submit(&mut self.runner, ctx, request);
            }
            Err(error) => {
                self.runner.stop();
                self.script_error = Some(error);
                self.runner.poll(ctx);
            }
        }
        if self.runner.key() != self.delivered
            && let Some(result) = self.runner.result()
        {
            self.delivered = self.runner.key();
            self.run_error = match result {
                Ok(outputs) => self.app.receive(outputs).err(),
                Err(error) => Some(error.clone()),
            };
        }
    }
    pub fn running(&self) -> bool {
        self.runner.running()
    }
    /// Private runs start in `folder`, so function files there are found.
    pub fn set_folder(&mut self, folder: Option<&std::path::Path>) {
        if self.runner.folder.as_deref() != folder {
            self.delivered = None;
            self.run_error = None;
        }
        self.runner.set_folder(folder);
    }
    pub fn status(&self) -> (String, AppStatus) {
        if let Some(error) = &self.script_error {
            return (error.clone(), AppStatus::Error);
        }
        if let Some(hint) = self.app.idle_hint() {
            return (hint.to_owned(), AppStatus::Plain);
        }
        if self.runner.running() && self.runner.result().is_none() {
            return (
                format!("{}: computing…", self.app.kind().label()),
                AppStatus::Plain,
            );
        }
        if let Some(error) = &self.run_error {
            return (error.clone(), AppStatus::Error);
        }
        match self.app.summary() {
            Some(text) => (text, AppStatus::Success),
            None => (self.app.kind().label().to_owned(), AppStatus::Plain),
        }
    }
    /// Export and Generate script need an idle workspace and a successful
    /// run of the current script, so they reproduce what the app shows.
    fn ready(&self, ws: &Snapshot, busy: bool) -> bool {
        !busy
            && !self.saving
            && self.script_error.is_none()
            && self.run_error.is_none()
            && !self.running()
            && self.runner.result().is_some_and(Result::is_ok)
            && self.app.script(ws).is_ok()
    }
    /// The Export action: the app's script, run in the workspace. If it
    /// fails, the exported names get their earlier values back (or are
    /// cleared) and the error is shown, so a failed Export writes no outputs.
    pub fn export(&mut self, ws: &Snapshot) -> Option<Action> {
        let script = self.app.export_code(ws).ok()?;
        let names = self.app.exported(ws);
        self.message = format!("Exported {} to the workspace", names.join(", "));
        Some(Action::ExportScript(all_or_nothing(&script, &names)))
    }
    /// The Generate script action: header plus the same script.
    pub fn generate(&mut self, ws: &Snapshot, unix_seconds: u64) -> Option<Action> {
        let script = self.app.script(ws).ok()?;
        let kind = self.app.kind();
        Some(Action::SaveAppScript(
            kind,
            saved_script(kind, unix_seconds, &script),
        ))
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        ws: &Snapshot,
        busy: bool,
        action: &mut Option<Action>,
    ) {
        self.refresh(ui.ctx(), ws);
        let ready = self.ready(ws, busy);
        let kind = self.app.kind();
        let caption = self.app.main_caption();
        let mut clicked = None;
        three_areas(ui, kind, caption, |ui, area| match area {
            Area::Inputs => self.app.inputs_ui(ui, ws),
            Area::Main => self.app.main_ui(ui),
            Area::Output => {
                result_block(ui, &self.app.results());
                ui.add_space(crate::tokens::M);
                self.app.names_ui(ui, ws);
                ui.add_space(crate::tokens::S);
                clicked = output_buttons_labelled(ui, ready, self.app.export_label());
                if !self.message.is_empty() {
                    ui.add_space(crate::tokens::S);
                    crate::tokens::helper(ui, &self.message);
                }
            }
        });
        *action = match clicked {
            Some(OutputClick::Export) => self.export(ws),
            Some(OutputClick::Script) => self.generate(ws, now()),
            None => action.take(),
        };
    }
}

// ---------------------------------------------------------------------------
// Layout

/// Width of the Inputs and Output columns.
pub const COLUMN: f32 = 280.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Inputs,
    Main,
    Output,
}

fn caption(ui: &mut egui::Ui, text: &str) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .font(crate::panels::caption_semibold(ui))
            .color(p.text_muted),
    );
    ui.add_space(crate::tokens::S);
}

/// INPUTS (280 px, scrolls) | main view (fills) | OUTPUT (280 px). A host
/// too narrow for three areas puts OUTPUT under the main view.
pub fn three_areas(
    ui: &mut egui::Ui,
    kind: AppKind,
    main_caption: &str,
    mut add: impl FnMut(&mut egui::Ui, Area),
) {
    use crate::panels::PaneBorder;
    let ctx = ui.ctx().clone();
    let p = crate::tokens::palette(&ctx);
    let id = |part: &str| egui::Id::new(("app_area", kind.label(), part.to_owned()));
    let frame = egui::Frame::new()
        .fill(p.surface_sunken)
        .inner_margin(crate::tokens::L as i8);
    let narrow = ui.available_width() < 2. * COLUMN + 360.;
    egui::Panel::left(id("inputs"))
        .exact_size(COLUMN)
        .resizable(false)
        .frame(frame)
        .show_pane(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(id("inputs_scroll"))
                .auto_shrink([false, false])
                .show_viewport_themed(ui, |ui, viewport| {
                    ui.spacing_mut().interact_size.y = crate::tokens::CONTROL;
                    caption(ui, "INPUTS");
                    add(ui, Area::Inputs);
                    if ui.rect_contains_pointer(ui.clip_rect()) {
                        for (key, direction) in
                            [(egui::Key::PageDown, -1.), (egui::Key::PageUp, 1.)]
                        {
                            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                                ui.scroll_with_delta(egui::vec2(0., direction * viewport.height()));
                            }
                        }
                    }
                });
        });
    if narrow {
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(id("compact_scroll"))
                .auto_shrink([false, false])
                .show_themed(ui, |ui| {
                    caption(ui, main_caption);
                    let size = egui::vec2(ui.available_width(), 320.);
                    ui.allocate_ui(size, |ui| {
                        ui.set_min_size(size);
                        add(ui, Area::Main);
                    });
                    ui.add_space(crate::tokens::L);
                    caption(ui, "OUTPUT");
                    add(ui, Area::Output);
                });
        });
        return;
    }
    egui::Panel::right(id("output"))
        .exact_size(COLUMN)
        .resizable(false)
        .frame(frame)
        .show_pane(ui, |ui| {
            // A vertical-only ScrollArea expands horizontally to its widest widget.
            // Keep the viewport fixed even for intrinsically wide output tables.
            egui::ScrollArea::both()
                .id_salt(id("output_scroll"))
                .auto_shrink([false, false])
                .show_themed(ui, |ui| {
                    ui.spacing_mut().interact_size.y = crate::tokens::CONTROL;
                    caption(ui, "OUTPUT");
                    add(ui, Area::Output);
                });
        });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        caption(ui, main_caption);
        add(ui, Area::Main);
    });
}

/// The Output column's result text: label and value per row.
pub fn result_block(ui: &mut egui::Ui, rows: &[(&'static str, String)]) {
    let p = crate::tokens::palette(ui.ctx());
    let width = ui.available_width();
    let content_width = (width - crate::tokens::S).max(1.);
    let font = egui::FontSelection::Default.resolve(ui.style());
    // Leave room for actual values. When both columns contain long text,
    // reserve half for the value and wrap the label within the rest.
    let mut reserved = 0.0_f32;
    for (_, value) in rows {
        reserved = reserved.max(
            ui.painter()
                .layout_no_wrap(value.clone(), font.clone(), p.text)
                .size()
                .x,
        );
        if reserved >= content_width / 2. {
            reserved = content_width / 2.;
            break;
        }
    }
    let label_limit = content_width - reserved;
    egui::Grid::new(ui.id().with("result_block"))
        .num_columns(2)
        .min_col_width(0.)
        .max_col_width(width)
        .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(ui.painter().layout(
                    (*label).into(),
                    font.clone(),
                    p.text_muted,
                    label_limit,
                ));
                ui.add(egui::Label::new(egui::RichText::new(value).color(p.text)).wrap());
                ui.end_row();
            }
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputClick {
    Export,
    Script,
}

/// "Export to workspace" and "Generate script".
#[allow(dead_code)]
pub fn output_buttons(ui: &mut egui::Ui, ready: bool) -> Option<OutputClick> {
    output_buttons_labelled(ui, ready, "Export to workspace")
}

/// The Output buttons with the app's caption for Export.
pub fn output_buttons_labelled(
    ui: &mut egui::Ui,
    ready: bool,
    export: &str,
) -> Option<OutputClick> {
    const WAIT: &str = "Available once the current inputs computed without an error \
                        and no command is running";
    let wide = egui::vec2(ui.available_width(), crate::tokens::CONTROL);
    let mut clicked = None;
    if ui
        .add_enabled(ready, egui::Button::new(export).min_size(wide))
        .on_disabled_hover_text(WAIT)
        .clicked()
    {
        clicked = Some(OutputClick::Export);
    }
    if ui
        .add_enabled(ready, egui::Button::new("Generate script").min_size(wide))
        .on_disabled_hover_text(WAIT)
        .clicked()
    {
        clicked = Some(OutputClick::Script);
    }
    clicked
}

/// One labelled input row: label above, control below.
pub fn field(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(label);
    add(ui);
    ui.add_space(crate::tokens::S);
}

/// Validate a field in the same assignment context as generated scripts. The
/// closing delimiter and semicolon must survive parsing: comments and statement
/// separators cannot escape into the rest of the script.
pub(crate) fn single_expression(text: &str) -> Result<(), String> {
    use matrust::syntax::StatementKind;
    if text.trim().is_empty() {
        return Err("Enter a variable name or expression".into());
    }
    let script = format!("app_expression = ({text});");
    let statements =
        matrust::syntax::parse(&script).map_err(|e| format!("Use exactly one expression: {e}"))?;
    if let [statement] = statements.as_slice()
        && let StatementKind::Evaluate(evaluation) = &statement.kind
        && evaluation.targets == [Some("app_expression".into())]
        && evaluation.quiet
    {
        return Ok(());
    }
    Err("Use exactly one expression".into())
}

/// Every app uses this check for names written by Export, including setup names.
pub(crate) fn check_export_name(name: &str) -> Result<(), String> {
    if matches!(
        name,
        "mrexp_old" | "mrexp_had" | "matrust_export_error" | "mrexp_complete"
    ) {
        return Err(format!(
            "{name} is reserved by workspace export; choose another name"
        ));
    }
    Ok(())
}

/// A field-local validation message, sharing the app's normal error styling.
pub(crate) fn expression_message(ui: &mut egui::Ui, result: Result<(), String>) {
    if let Err(error) = result {
        let p = crate::tokens::palette(ui.ctx());
        ui.label(
            egui::RichText::new(error)
                .size(crate::tokens::CAPTION)
                .color(p.error),
        );
    }
}

/// An editable output name; shows whether it is valid or replaces a variable.
pub fn name_field(ui: &mut egui::Ui, label: &str, name: &mut String, ws: &Snapshot) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label(label);
    ui.add(
        crate::tokens::input(egui::TextEdit::singleline(name).char_limit(63))
            .desired_width(ui.available_width()),
    );
    if !matrust::files::valid_name(name) {
        ui.label(
            egui::RichText::new("Use a valid variable name")
                .size(crate::tokens::CAPTION)
                .color(p.error),
        );
    } else if let Err(error) = check_export_name(name) {
        expression_message(ui, Err(error));
    } else if ws.contains(name) {
        ui.label(
            egui::RichText::new(format!("Replaces the workspace variable {name}"))
                .size(crate::tokens::CAPTION)
                .color(p.warning),
        );
    }
    ui.add_space(crate::tokens::XS);
}

// ---------------------------------------------------------------------------
// The apps of the Apps window

type SaveRequest = (AppKind, String, Option<Array>, Option<(String, Array)>);

/// Open apps, their state, and the workspace view they share.
pub struct Apps {
    pub basic: Host<basic_fitting::BasicFitting>,
    open: Vec<AppKind>,
    pub curve: Host<curve_fitter::CurveFitter>,
    pub pid: Host<pid_tuner::PidTuner>,
    pub signal: Host<signal_analyzer::SignalAnalyzer>,
    pub import: Host<import_tool::ImportTool>,
    pub control: Host<control_system_designer::ControlSystemDesigner>,
    pub lsa: Host<linear_system_analyzer::LinearSystemAnalyzer>,
    pub distribution: Host<distribution_fitter::DistributionFitter>,
    pub explorer: Host<distribution_explorer::DistributionExplorer>,
    pub cleaner: Host<data_cleaner::DataCleaner>,
    pub optimize: Host<optimize_app::Optimize>,
    pub ode: Host<solve_ode_app::SolveOde>,
    pub statistics: Host<data_statistics::DataStatistics>,
    pub image: Host<image_viewer::ImageViewer>,
    pub video: Host<video_viewer::VideoViewer>,
    pub window: Host<window_designer::WindowDesigner>,
    pub spline: Host<spline_tool::SplineTool>,
    pub color: Host<color_thresholder::ColorThresholder>,
    pub classification: classification_learner::ClassificationLearner,
    pub regression: Host<regression_learner::RegressionLearner>,
    pub segmenter: Host<image_segmenter::ImageSegmenter>,
    pub region: Host<image_region_analyzer::ImageRegionAnalyzer>,
    pub batch: image_batch_processor::ImageBatchProcessor,
    pub reducer: Host<model_reducer::ModelReducer>,
    pub sysid: Host<system_identification::SystemIdentification>,
    pub filter: Host<filter_visualizer::FilterVisualizer>,
    pub wavelet: Host<wavelet_denoiser::WaveletDenoiser>,
    pub mra: Host<multires_analyzer::MultiresAnalyzer>,
    pub wavelet_tf: Host<wavelet_tf_analyzer::WaveletTfAnalyzer>,
    pub comparison: Host<comparison_tool::ComparisonTool>,
    pub fuzzy: Host<fuzzy_designer::FuzzyDesigner>,
    pub ber: Host<ber_analysis::BerAnalysis>,
    pub waveform: Host<waveform_generator::WaveformGenerator>,
    pub profiler: Host<profiler_app::Profiler>,
    pub neural_fitting: Host<neural_net_apps::NeuralNet>,
    pub neural_pattern: Host<neural_net_apps::NeuralNet>,
    pub neural_clustering: Host<neural_net_apps::NeuralNet>,
    pub neural_time_series: Host<neural_net_apps::NeuralNet>,
    pub mpc: Host<mpc_designer::MpcDesigner>,
    pub dicom: Host<dicom_browser::DicomBrowser>,
    pub registration: Host<registration_estimator::RegistrationEstimator>,
    pub camera_calibrator: Host<camera_calibrator::CameraCalibrator>,
    pub volume_viewer: Host<volume_viewer::VolumeViewer>,
    pub app_designer: app_designer::AppDesigner,
    pub pde_modeler: Host<pde_modeler::PdeModeler>,
    pub econ_modeler: Host<econometric_modeler::EconometricModeler>,
    pub workspace: Snapshot,
    /// A Generate script click waiting for its file name.
    save_request: Option<SaveRequest>,
    /// The app whose script the worker is saving.
    saving: Option<AppKind>,
    /// Headless tests choose the file instead of the dialog.
    #[cfg(test)]
    pub test_save_path: Option<PathBuf>,
    /// Headless tests answer the Import Tool's file picker: the answer and
    /// how often the picker was shown.
    #[cfg(test)]
    pub test_pick: Option<PathBuf>,
    #[cfg(test)]
    pub test_picks: usize,
}
impl Default for Apps {
    fn default() -> Self {
        Self {
            open: Vec::new(),
            basic: Host::new(basic_fitting::BasicFitting::default()),
            curve: Host::new(curve_fitter::CurveFitter::default()),
            pid: Host::new(pid_tuner::PidTuner::default()),
            signal: Host::new(signal_analyzer::SignalAnalyzer::default()),
            import: Host::new(import_tool::ImportTool::default()),
            control: Host::new(control_system_designer::ControlSystemDesigner::default()),
            lsa: Host::new(linear_system_analyzer::LinearSystemAnalyzer::default()),
            distribution: Host::new(distribution_fitter::DistributionFitter::default()),
            explorer: Host::new(distribution_explorer::DistributionExplorer::default()),
            cleaner: Host::new(data_cleaner::DataCleaner::default()),
            optimize: Host::new(optimize_app::Optimize::default()),
            ode: Host::new(solve_ode_app::SolveOde::default()),
            statistics: Host::new(data_statistics::DataStatistics::default()),
            image: Host::new(image_viewer::ImageViewer::default()),
            video: Host::new(video_viewer::VideoViewer::default()),
            window: Host::new(window_designer::WindowDesigner::default()),
            spline: Host::new(spline_tool::SplineTool::default()),
            color: Host::new(color_thresholder::ColorThresholder::default()),
            classification: classification_learner::ClassificationLearner::default(),
            regression: Host::new(regression_learner::RegressionLearner::default()),
            segmenter: Host::new(image_segmenter::ImageSegmenter::default()),
            region: Host::new(image_region_analyzer::ImageRegionAnalyzer::default()),
            batch: image_batch_processor::ImageBatchProcessor::default(),
            reducer: Host::new(model_reducer::ModelReducer::default()),
            sysid: Host::new(system_identification::SystemIdentification::default()),
            filter: Host::new(filter_visualizer::FilterVisualizer::default()),
            wavelet: Host::new(wavelet_denoiser::WaveletDenoiser::default()),
            mra: Host::new(multires_analyzer::MultiresAnalyzer::default()),
            wavelet_tf: Host::new(wavelet_tf_analyzer::WaveletTfAnalyzer::default()),
            comparison: Host::new(comparison_tool::ComparisonTool::default()),
            fuzzy: Host::new(fuzzy_designer::FuzzyDesigner::default()),
            ber: Host::new(ber_analysis::BerAnalysis::default()),
            waveform: Host::new(waveform_generator::WaveformGenerator::default()),
            profiler: Host::new(profiler_app::Profiler::default()),
            neural_fitting: Host::new(neural_net_apps::NeuralNet::new(
                neural_net_apps::Mode::Fitting,
            )),
            neural_pattern: Host::new(neural_net_apps::NeuralNet::new(
                neural_net_apps::Mode::PatternRecognition,
            )),
            neural_clustering: Host::new(neural_net_apps::NeuralNet::new(
                neural_net_apps::Mode::Clustering,
            )),
            neural_time_series: Host::new(neural_net_apps::NeuralNet::new(
                neural_net_apps::Mode::TimeSeries,
            )),
            mpc: Host::new(mpc_designer::MpcDesigner::default()),
            dicom: Host::new(dicom_browser::DicomBrowser::default()),
            registration: Host::new(registration_estimator::RegistrationEstimator::default()),
            camera_calibrator: Host::new(camera_calibrator::CameraCalibrator::default()),
            volume_viewer: Host::new(volume_viewer::VolumeViewer::default()),
            app_designer: app_designer::AppDesigner::default(),
            pde_modeler: Host::new(pde_modeler::PdeModeler::default()),
            econ_modeler: Host::new(econometric_modeler::EconometricModeler::default()),
            workspace: Snapshot::default(),
            save_request: None,
            saving: None,
            #[cfg(test)]
            test_save_path: None,
            #[cfg(test)]
            test_pick: None,
            #[cfg(test)]
            test_picks: 0,
        }
    }
}
impl Apps {
    pub fn is_open(&self, kind: AppKind) -> bool {
        self.open.contains(&kind)
    }
    pub fn any_open(&self) -> bool {
        !self.open.is_empty()
    }
    pub fn open(&mut self, kind: AppKind) {
        if kind.is_built() && !self.is_open(kind) {
            self.open.push(kind);
        }
    }
    /// Closing keeps the app's inputs for the next open; its run stops.
    pub fn close(&mut self, kind: AppKind) {
        self.open.retain(|k| *k != kind);
        match kind {
            AppKind::MpcDesigner => mpc_designer::stop(&mut self.mpc),
            AppKind::DicomBrowser => dicom_browser::stop(&mut self.dicom),
            AppKind::RegistrationEstimator => registration_estimator::close(&mut self.registration),
            AppKind::CameraCalibrator => camera_calibrator::close(&mut self.camera_calibrator),
            AppKind::VolumeViewer => volume_viewer::close(&mut self.volume_viewer),
            AppKind::AppDesigner => {
                self.app_designer.close_confirm = false;
                self.app_designer.placing = None;
                self.app_designer.dragging = None;
            }
            AppKind::PdeModeler => pde_modeler::close(&mut self.pde_modeler),
            AppKind::EconometricModeler => econometric_modeler::stop(&mut self.econ_modeler),
            AppKind::CurveFitter => {
                self.curve.runner.stop();
                self.curve.app.stop();
            }
            AppKind::FuzzyLogicDesigner => fuzzy_designer::stop(&mut self.fuzzy),
            AppKind::BitErrorRateAnalysis => {
                self.ber.runner.stop();
                self.ber.app.running = false;
            }
            AppKind::WirelessWaveformGenerator => {
                self.waveform.runner.stop();
                self.waveform.app.running = false;
            }
            AppKind::ComparisonTool => self.comparison.runner.stop(),
            AppKind::NeuralNetFitting => self.neural_fitting.runner.stop(),
            AppKind::NeuralNetPatternRecognition => self.neural_pattern.runner.stop(),
            AppKind::NeuralNetClustering => self.neural_clustering.runner.stop(),
            AppKind::NeuralNetTimeSeries => self.neural_time_series.runner.stop(),
            AppKind::Profiler => {
                self.profiler.runner.stop();
                self.profiler.app.stop();
            }
            AppKind::BasicFitting => self.basic.runner.stop(),
            AppKind::PidTuner => self.pid.runner.stop(),
            AppKind::SignalAnalyzer => self.signal.runner.stop(),
            AppKind::ImportTool => self.import.runner.stop(),
            AppKind::ControlSystemDesigner => self.control.runner.stop(),
            AppKind::LinearSystemAnalyzer => {
                self.lsa.runner.stop();
                self.lsa.app.stop();
            }
            AppKind::DistributionFitter => self.distribution.runner.stop(),
            AppKind::DistributionExplorer => self.explorer.runner.stop(),
            AppKind::DataCleaner => self.cleaner.runner.stop(),
            AppKind::Optimize => {
                self.optimize.runner.stop();
                self.optimize.app.running = false;
            }
            AppKind::SolveOde => self.ode.runner.stop(),
            AppKind::DataStatistics => self.statistics.runner.stop(),
            AppKind::RegressionLearner => self.regression.app.stop(),
            AppKind::ImageViewer => self.image.runner.stop(),
            AppKind::VideoViewer => video_viewer::stop(&mut self.video),
            AppKind::WindowDesigner => self.window.runner.stop(),
            AppKind::SplineTool => self.spline.runner.stop(),
            AppKind::ColorThresholder => self.color.runner.stop(),
            AppKind::ClassificationLearner => self.classification.stop(),
            AppKind::ImageSegmenter => self.segmenter.runner.stop(),
            AppKind::ImageRegionAnalyzer => self.region.runner.stop(),
            AppKind::ImageBatchProcessor => self.batch.close(),
            AppKind::ModelReducer => {
                self.reducer.runner.stop();
                self.reducer.app.stop();
            }
            AppKind::SystemIdentification => self.sysid.runner.stop(),
            AppKind::FilterVisualizer => self.filter.runner.stop(),
            AppKind::WaveletDenoiser => self.wavelet.runner.stop(),
            AppKind::MultiresAnalyzer => self.mra.runner.stop(),
            AppKind::WaveletTfAnalyzer => self.wavelet_tf.runner.stop(),
        }
    }
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        kind: AppKind,
        busy: bool,
        action: &mut Option<Action>,
    ) {
        match kind {
            AppKind::DicomBrowser => {
                dicom_browser::show(&mut self.dicom, ui, &self.workspace, busy, action)
            }
            AppKind::RegistrationEstimator => registration_estimator::show(
                &mut self.registration,
                ui,
                &self.workspace,
                busy,
                action,
            ),
            AppKind::VolumeViewer => {
                volume_viewer::show(&mut self.volume_viewer, ui, &self.workspace, busy, action)
            }
            AppKind::AppDesigner => self.app_designer.show(ui, busy, action),
            AppKind::PdeModeler => {
                pde_modeler::show(&mut self.pde_modeler, ui, &self.workspace, busy, action)
            }
            AppKind::CameraCalibrator => camera_calibrator::show(
                &mut self.camera_calibrator,
                ui,
                &self.workspace,
                busy,
                action,
            ),
            AppKind::MpcDesigner => {
                mpc_designer::show(&mut self.mpc, ui, &self.workspace, busy, action)
            }
            AppKind::EconometricModeler => {
                econometric_modeler::show(&mut self.econ_modeler, ui, &self.workspace, busy, action)
            }
            AppKind::ComparisonTool => {
                comparison_tool::show(&mut self.comparison, ui, &self.workspace, busy, action)
            }
            AppKind::ClassificationLearner => {
                self.classification.show(ui, &self.workspace, busy, action)
            }
            AppKind::BasicFitting => {
                basic_fitting::show(&mut self.basic, ui, &self.workspace, busy, action)
            }
            AppKind::RegressionLearner => {
                regression_learner::show(&mut self.regression, ui, &self.workspace, busy, action)
            }
            AppKind::BitErrorRateAnalysis => {
                ber_analysis::show(&mut self.ber, ui, &self.workspace, busy, action)
            }
            AppKind::WirelessWaveformGenerator => {
                waveform_generator::show(&mut self.waveform, ui, &self.workspace, busy, action)
            }
            AppKind::FuzzyLogicDesigner => {
                fuzzy_designer::show(&mut self.fuzzy, ui, &self.workspace, busy, action);
            }
            AppKind::Profiler => {
                profiler_app::show(&mut self.profiler, ui, &self.workspace, busy, action);
            }
            AppKind::CurveFitter => self.curve.show(ui, &self.workspace, busy, action),
            AppKind::NeuralNetFitting => {
                neural_net_apps::show(&mut self.neural_fitting, ui, &self.workspace, busy, action)
            }
            AppKind::NeuralNetPatternRecognition => {
                neural_net_apps::show(&mut self.neural_pattern, ui, &self.workspace, busy, action)
            }
            AppKind::NeuralNetClustering => neural_net_apps::show(
                &mut self.neural_clustering,
                ui,
                &self.workspace,
                busy,
                action,
            ),
            AppKind::NeuralNetTimeSeries => neural_net_apps::show(
                &mut self.neural_time_series,
                ui,
                &self.workspace,
                busy,
                action,
            ),
            AppKind::PidTuner => self.pid.show(ui, &self.workspace, busy, action),
            AppKind::SignalAnalyzer => self.signal.show(ui, &self.workspace, busy, action),
            AppKind::VideoViewer => {
                video_viewer::show(&mut self.video, ui, &self.workspace, busy, action)
            }
            AppKind::ImageViewer => {
                image_viewer::show(&mut self.image, ui, &self.workspace, busy, action)
            }
            AppKind::WindowDesigner => self.window.show(ui, &self.workspace, busy, action),
            AppKind::SplineTool => {
                self.spline.app.running = self.spline.running();
                self.spline.show(ui, &self.workspace, busy, action)
            }
            AppKind::ColorThresholder => {
                color_thresholder::show(&mut self.color, ui, &self.workspace, busy, action)
            }
            AppKind::ImportTool => self.import.show(ui, &self.workspace, busy, action),
            AppKind::ControlSystemDesigner => self.control.show(ui, &self.workspace, busy, action),
            AppKind::LinearSystemAnalyzer => self.lsa.show(ui, &self.workspace, busy, action),
            AppKind::DistributionFitter => {
                self.distribution.show(ui, &self.workspace, busy, action)
            }
            AppKind::DistributionExplorer => {
                distribution_explorer::show(&mut self.explorer, ui, &self.workspace, busy, action)
            }
            AppKind::DataCleaner => self.cleaner.show(ui, &self.workspace, busy, action),
            AppKind::Optimize => {
                self.optimize.set_folder(self.workspace.folder.as_deref());
                self.optimize.app.running = self.optimize.running();
                self.optimize.show(ui, &self.workspace, busy, action)
            }
            AppKind::SolveOde => {
                // The view offers Stop and a stiffness hint while solving.
                let running = self.ode.running();
                self.ode.app.set_running(running);
                self.ode.show(ui, &self.workspace, busy, action)
            }
            AppKind::DataStatistics => {
                data_statistics::show(&mut self.statistics, ui, &self.workspace, busy, action)
            }
            AppKind::ImageSegmenter => {
                // The Inputs column offers Stop while the steps compute.
                self.segmenter.app.running = self.segmenter.running();
                self.segmenter.show(ui, &self.workspace, busy, action)
            }
            AppKind::ImageRegionAnalyzer => {
                self.region.app.running = self.region.running();
                self.region.show(ui, &self.workspace, busy, action)
            }
            AppKind::ImageBatchProcessor => self.batch.show(ui, &self.workspace, busy, action),
            AppKind::ModelReducer => {
                // The Inputs column offers Stop while the reductions compute.
                self.reducer.app.running = self.reducer.running();
                self.reducer.show(ui, &self.workspace, busy, action)
            }
            AppKind::SystemIdentification => {
                // The main view offers Stop while the models are estimated.
                self.sysid.app.running = self.sysid.running();
                self.sysid.show(ui, &self.workspace, busy, action)
            }
            AppKind::FilterVisualizer => {
                filter_visualizer::show(&mut self.filter, ui, &self.workspace, busy, action)
            }
            AppKind::WaveletDenoiser => {
                // The Inputs column offers Stop while the signal is denoised.
                self.wavelet.app.running = self.wavelet.running();
                self.wavelet.show(ui, &self.workspace, busy, action)
            }
            AppKind::MultiresAnalyzer => {
                // The main view offers Stop while the decomposition runs.
                self.mra.app.running = self.mra.running();
                self.mra.show(ui, &self.workspace, busy, action)
            }
            AppKind::WaveletTfAnalyzer => {
                // The Inputs column offers Stop while the transform computes.
                self.wavelet_tf.app.running = self.wavelet_tf.running();
                self.wavelet_tf.show(ui, &self.workspace, busy, action);
                if std::mem::take(&mut self.wavelet_tf.app.stop_requested) {
                    self.wavelet_tf.runner.stop();
                }
            }
        }
    }
    pub fn status(&self, kind: AppKind) -> Option<(String, AppStatus)> {
        match kind {
            AppKind::MpcDesigner => Some(self.mpc.status()),
            AppKind::DicomBrowser => Some(self.dicom.status()),
            AppKind::RegistrationEstimator => Some(self.registration.status()),
            AppKind::CameraCalibrator => Some(self.camera_calibrator.status()),
            AppKind::VolumeViewer => Some(self.volume_viewer.status()),
            AppKind::AppDesigner => None,
            AppKind::PdeModeler => Some(self.pde_modeler.status()),
            AppKind::EconometricModeler => Some(self.econ_modeler.status()),
            AppKind::ComparisonTool => Some(self.comparison.status()),
            AppKind::FuzzyLogicDesigner => Some(self.fuzzy.status()),
            AppKind::BitErrorRateAnalysis => Some(self.ber.status()),
            AppKind::WirelessWaveformGenerator => Some(self.waveform.status()),
            AppKind::Profiler => Some(self.profiler.status()),
            AppKind::NeuralNetFitting => Some(self.neural_fitting.status()),
            AppKind::NeuralNetPatternRecognition => Some(self.neural_pattern.status()),
            AppKind::NeuralNetClustering => Some(self.neural_clustering.status()),
            AppKind::NeuralNetTimeSeries => Some(self.neural_time_series.status()),
            AppKind::BasicFitting => Some(self.basic.status()),
            AppKind::CurveFitter => Some(self.curve.status()),
            AppKind::PidTuner => Some(self.pid.status()),
            AppKind::SignalAnalyzer => Some(self.signal.status()),
            AppKind::ImportTool => Some(self.import.status()),
            AppKind::ControlSystemDesigner => Some(self.control.status()),
            AppKind::LinearSystemAnalyzer => Some(self.lsa.status()),
            AppKind::DistributionFitter => Some(self.distribution.status()),
            AppKind::DistributionExplorer => Some(self.explorer.status()),
            AppKind::DataCleaner => Some(self.cleaner.status()),
            AppKind::Optimize => Some(self.optimize.status()),
            AppKind::SolveOde => Some(self.ode.status()),
            AppKind::DataStatistics => Some(self.statistics.status()),
            AppKind::RegressionLearner => Some(regression_learner::status(&self.regression)),
            AppKind::ImageViewer => Some(self.image.status()),
            AppKind::VideoViewer => Some(self.video.status()),
            AppKind::WindowDesigner => Some(self.window.status()),
            AppKind::SplineTool => Some(self.spline.status()),
            AppKind::ColorThresholder => Some(self.color.status()),
            AppKind::ClassificationLearner => Some(self.classification.status()),
            AppKind::ImageSegmenter => Some(self.segmenter.status()),
            AppKind::ImageRegionAnalyzer => Some(self.region.status()),
            AppKind::ImageBatchProcessor => Some(self.batch.status()),
            AppKind::ModelReducer => Some(self.reducer.status()),
            AppKind::SystemIdentification => Some(self.sysid.status()),
            AppKind::FilterVisualizer => Some(self.filter.status()),
            AppKind::WaveletDenoiser => Some(self.wavelet.status()),
            AppKind::MultiresAnalyzer => Some(self.mra.status()),
            AppKind::WaveletTfAnalyzer => Some(self.wavelet_tf.status()),
        }
    }
    /// The Apps action row: the app's last message (its commands live in OUTPUT).
    pub fn commands(&mut self, ui: &mut egui::Ui, kind: AppKind) {
        let message = match kind {
            AppKind::MpcDesigner => self.mpc.message.as_str(),
            AppKind::DicomBrowser => self.dicom.message.as_str(),
            AppKind::RegistrationEstimator => self.registration.message.as_str(),
            AppKind::CameraCalibrator => self.camera_calibrator.message.as_str(),
            AppKind::VolumeViewer => "",
            AppKind::AppDesigner => "",
            AppKind::PdeModeler => self.pde_modeler.message.as_str(),
            AppKind::EconometricModeler => self.econ_modeler.message.as_str(),
            AppKind::ComparisonTool => self.comparison.message.as_str(),
            AppKind::FuzzyLogicDesigner => self.fuzzy.message.as_str(),
            AppKind::BitErrorRateAnalysis => self.ber.message.as_str(),
            AppKind::WirelessWaveformGenerator => self.waveform.message.as_str(),
            AppKind::Profiler => self.profiler.message.as_str(),
            AppKind::NeuralNetFitting => self.neural_fitting.message.as_str(),
            AppKind::NeuralNetPatternRecognition => self.neural_pattern.message.as_str(),
            AppKind::NeuralNetClustering => self.neural_clustering.message.as_str(),
            AppKind::NeuralNetTimeSeries => self.neural_time_series.message.as_str(),
            AppKind::BasicFitting => self.basic.message.as_str(),
            AppKind::CurveFitter => self.curve.message.as_str(),
            AppKind::PidTuner => self.pid.message.as_str(),
            AppKind::SignalAnalyzer => self.signal.message.as_str(),
            AppKind::ImportTool => self.import.message.as_str(),
            AppKind::ControlSystemDesigner => self.control.message.as_str(),
            AppKind::LinearSystemAnalyzer => self.lsa.message.as_str(),
            AppKind::DistributionFitter => self.distribution.message.as_str(),
            AppKind::DistributionExplorer => self.explorer.message.as_str(),
            AppKind::DataCleaner => self.cleaner.message.as_str(),
            AppKind::Optimize => self.optimize.message.as_str(),
            AppKind::SolveOde => self.ode.message.as_str(),
            AppKind::DataStatistics => self.statistics.message.as_str(),
            AppKind::RegressionLearner => self.regression.message.as_str(),
            AppKind::ImageViewer => self.image.message.as_str(),
            AppKind::VideoViewer => self.video.message.as_str(),
            AppKind::WindowDesigner => self.window.message.as_str(),
            AppKind::SplineTool => self.spline.message.as_str(),
            AppKind::ColorThresholder => self.color.message.as_str(),
            AppKind::ClassificationLearner => self.classification.message.as_str(),
            AppKind::ImageSegmenter => self.segmenter.message.as_str(),
            AppKind::ImageRegionAnalyzer => self.region.message.as_str(),
            AppKind::ImageBatchProcessor => self.batch.message.as_str(),
            AppKind::ModelReducer => self.reducer.message.as_str(),
            AppKind::SystemIdentification => self.sysid.message.as_str(),
            AppKind::FilterVisualizer => self.filter.message.as_str(),
            AppKind::WaveletDenoiser => self.wavelet.message.as_str(),
            AppKind::MultiresAnalyzer => self.mra.message.as_str(),
            AppKind::WaveletTfAnalyzer => self.wavelet_tf.message.as_str(),
        };
        let p = crate::tokens::palette(ui.ctx());
        ui.add(egui::Label::new(egui::RichText::new(message).color(p.text_muted)).truncate());
    }
    fn set_saving(&mut self, kind: AppKind, saving: bool, message: String) {
        match kind {
            AppKind::DicomBrowser => {
                self.dicom.saving = saving;
                self.dicom.message = message;
            }
            AppKind::RegistrationEstimator => {
                self.registration.saving = saving;
                self.registration.message = message;
            }
            AppKind::VolumeViewer => {
                self.volume_viewer.saving = saving;
                self.volume_viewer.message = message;
            }
            AppKind::AppDesigner => {
                self.app_designer.saving = saving;
                if saving {
                    self.app_designer.set_notice(message);
                } else if !message.is_empty() {
                    self.app_designer.set_error(message);
                }
            }
            AppKind::PdeModeler => {
                self.pde_modeler.saving = saving;
                self.pde_modeler.message = message;
            }
            AppKind::CameraCalibrator => {
                self.camera_calibrator.saving = saving;
                self.camera_calibrator.message = message;
            }
            AppKind::MpcDesigner => {
                self.mpc.saving = saving;
                self.mpc.message = message;
            }
            AppKind::EconometricModeler => {
                self.econ_modeler.saving = saving;
                self.econ_modeler.message = message;
            }
            AppKind::ComparisonTool => {
                self.comparison.saving = saving;
                self.comparison.message = message;
            }
            AppKind::NeuralNetFitting => {
                self.neural_fitting.saving = saving;
                self.neural_fitting.message = message;
            }
            AppKind::NeuralNetPatternRecognition => {
                self.neural_pattern.saving = saving;
                self.neural_pattern.message = message;
            }
            AppKind::NeuralNetClustering => {
                self.neural_clustering.saving = saving;
                self.neural_clustering.message = message;
            }
            AppKind::NeuralNetTimeSeries => {
                self.neural_time_series.saving = saving;
                self.neural_time_series.message = message;
            }
            AppKind::BitErrorRateAnalysis => {
                self.ber.saving = saving;
                self.ber.message = message;
            }
            AppKind::WirelessWaveformGenerator => {
                self.waveform.saving = saving;
                self.waveform.message = message;
            }
            AppKind::FuzzyLogicDesigner => {
                self.fuzzy.saving = saving;
                self.fuzzy.message = message;
            }
            AppKind::Profiler => {
                self.profiler.saving = saving;
                self.profiler.message = message;
            }
            AppKind::ColorThresholder => {
                self.color.saving = saving;
                self.color.message = message;
            }
            AppKind::ClassificationLearner => {
                self.classification.saving = saving;
                self.classification.message = message;
            }
            AppKind::BasicFitting => {
                self.basic.saving = saving;
                self.basic.message = message;
            }
            AppKind::RegressionLearner => {
                self.regression.saving = saving;
                self.regression.message = message;
            }
            AppKind::VideoViewer => {
                self.video.saving = saving;
                self.video.message = message;
            }
            AppKind::ImageViewer => {
                self.image.saving = saving;
                self.image.message = message;
            }
            AppKind::SplineTool => {
                self.spline.saving = saving;
                self.spline.message = message;
            }
            AppKind::WindowDesigner => {
                self.window.saving = saving;
                self.window.message = message;
            }
            AppKind::CurveFitter => {
                self.curve.saving = saving;
                self.curve.message = message;
            }
            AppKind::PidTuner => {
                self.pid.saving = saving;
                self.pid.message = message;
            }
            AppKind::SignalAnalyzer => {
                self.signal.saving = saving;
                self.signal.message = message;
            }
            AppKind::ImportTool => {
                self.import.saving = saving;
                self.import.message = message;
            }
            AppKind::ControlSystemDesigner => {
                self.control.saving = saving;
                self.control.message = message;
            }
            AppKind::LinearSystemAnalyzer => {
                self.lsa.saving = saving;
                self.lsa.message = message;
            }
            AppKind::DistributionFitter => {
                self.distribution.saving = saving;
                self.distribution.message = message;
            }
            AppKind::DistributionExplorer => {
                self.explorer.saving = saving;
                self.explorer.message = message;
            }
            AppKind::DataStatistics => {
                self.statistics.saving = saving;
                self.statistics.message = message;
            }
            AppKind::DataCleaner => {
                self.cleaner.saving = saving;
                self.cleaner.message = message;
            }
            AppKind::Optimize => {
                self.optimize.saving = saving;
                self.optimize.message = message;
            }
            AppKind::SolveOde => {
                self.ode.saving = saving;
                self.ode.message = message;
            }
            AppKind::ImageSegmenter => {
                self.segmenter.saving = saving;
                self.segmenter.message = message;
            }
            AppKind::ImageRegionAnalyzer => {
                self.region.saving = saving;
                self.region.message = message;
            }
            AppKind::ImageBatchProcessor => {
                self.batch.saving = saving;
                self.batch.message = message;
            }
            AppKind::ModelReducer => {
                self.reducer.saving = saving;
                self.reducer.message = message;
            }
            AppKind::SystemIdentification => {
                self.sysid.saving = saving;
                self.sysid.message = message;
            }
            AppKind::FilterVisualizer => {
                self.filter.saving = saving;
                self.filter.message = message;
            }
            AppKind::WaveletDenoiser => {
                self.wavelet.saving = saving;
                self.wavelet.message = message;
            }
            AppKind::MultiresAnalyzer => {
                self.mra.saving = saving;
                self.mra.message = message;
            }
            AppKind::WaveletTfAnalyzer => {
                self.wavelet_tf.saving = saving;
                self.wavelet_tf.message = message;
            }
        }
    }
    /// The app whose save just finished, if the finished save was an app's.
    pub fn take_saving(&mut self) -> Option<AppKind> {
        self.saving.take()
    }
}

// ---------------------------------------------------------------------------
// Desktop glue: workspace view and Generate script

impl crate::Workspace {
    /// Keeps the open apps' view of the workspace current.
    pub(super) fn sync_app_workspace(&mut self) {
        if self.toolkits.apps.any_open() {
            self.toolkits.apps.workspace.sync(&self.variables);
            self.toolkits
                .apps
                .workspace
                .set_folder(Some(self.folder.clone()));
            self.toolkits.apps.basic.app.sync_figures(&self.all_figures);
            if self.toolkits.apps.is_open(AppKind::DataStatistics) {
                self.toolkits
                    .apps
                    .statistics
                    .app
                    .sync_figures(&self.all_figures);
            }
        }
    }

    /// Generate script: the file name is chosen at the end of the frame.
    pub(super) fn queue_app_script_save(&mut self, kind: AppKind, script: String) {
        self.queue_app_script_save_with_volume(kind, script, None, None);
    }

    pub(super) fn queue_app_script_save_with_volume(
        &mut self,
        kind: AppKind,
        script: String,
        volume: Option<Array>,
        labels: Option<(String, Array)>,
    ) {
        if self.busy || self.toolkits.apps.saving.is_some() {
            self.toolkits.apps.set_saving(
                kind,
                false,
                "Wait for the running command to finish, then generate the script".into(),
            );
            return;
        }
        self.toolkits.apps.save_request = Some((kind, script, volume, labels));
    }

    /// Asks for the file and saves through the worker (`atomic_write`), the
    /// Filter Designer's path.
    pub(super) fn finish_app_script_save(&mut self, ctx: &egui::Context) {
        if ctx.will_discard() {
            return;
        }
        let Some((kind, script, volume, labels)) = self.toolkits.apps.save_request.take() else {
            return;
        };
        if self.busy {
            return;
        }
        let chosen_report = if kind == AppKind::ComparisonTool {
            self.toolkits.apps.comparison.app.report_path.take()
        } else if kind == AppKind::AppDesigner {
            let save_as = std::mem::take(&mut self.toolkits.apps.app_designer.save_as_requested);
            if let Some(pending) = self.toolkits.apps.app_designer.pending_save_path.take() {
                let resolved = if pending.is_absolute() {
                    pending
                } else {
                    self.folder.join(pending)
                };
                Some(resolved)
            } else if save_as {
                None
            } else {
                self.toolkits.apps.app_designer.file_path.clone()
            }
        } else {
            None
        };
        let report = kind == AppKind::ComparisonTool && chosen_report.is_some();
        let path = if let Some(path) = chosen_report {
            path
        } else if kind == AppKind::AppDesigner {
            match self.pick_app_designer_save_path() {
                Ok(path) => path,
                Err(reason) => {
                    let default_path = self.folder.join(format!(
                        "{}.m",
                        self.toolkits.apps.app_designer.model.class_name
                    ));
                    self.toolkits
                        .apps
                        .app_designer
                        .open_save_path_fallback(default_path, reason);
                    return;
                }
            }
        } else if let Some(path) = self.pick_app_script_path(kind) {
            path
        } else {
            return;
        };
        let script = if kind == AppKind::AppDesigner {
            match app_designer::prepare_save_code(&mut self.toolkits.apps.app_designer, &path) {
                Ok(code) => code,
                Err(err) => {
                    self.toolkits.apps.app_designer.on_save_error(err);
                    return;
                }
            }
        } else {
            script
        };
        self.submit(crate::worker::Job::SaveDesign(Box::new(
            crate::designer_output::Save {
                script,
                path,
                volume,
                labels,
            },
        )));
        if self.busy {
            self.toolkits.apps.saving = Some(kind);
            if report {
                self.toolkits
                    .apps
                    .set_saving(kind, true, "Saving the comparison report…".into());
                self.status = "Saving the comparison report…".into();
            } else {
                self.toolkits
                    .apps
                    .set_saving(kind, true, "Saving the script…".into());
                self.status = format!("Saving the {} script…", kind.label());
            }
        } else {
            let error = self.status.clone();
            self.toolkits.apps.set_saving(kind, false, error);
        }
    }

    /// The worker finished saving an app's script.
    pub(super) fn complete_app_script_save(
        &mut self,
        kind: AppKind,
        result: matrust::Result<PathBuf>,
    ) {
        if kind == AppKind::ComparisonTool {
            self.toolkits.apps.comparison.run_error = result.as_ref().err().cloned();
        }
        if kind == AppKind::AppDesigner {
            match result {
                Ok(path) => {
                    self.toolkits.apps.app_designer.on_saved(path.clone());
                    self.status = self.toolkits.apps.app_designer.message.clone();
                    let run_after =
                        std::mem::take(&mut self.toolkits.apps.app_designer.run_after_save);
                    let close_after =
                        std::mem::take(&mut self.toolkits.apps.app_designer.close_after_save);
                    if run_after {
                        let class_name = self.toolkits.apps.app_designer.model.class_name.clone();
                        let dir = path
                            .parent()
                            .map(std::path::Path::to_path_buf)
                            .unwrap_or_else(|| self.folder.clone());
                        self.submit(crate::worker::Job::ScriptInFolder(class_name, dir));
                    }
                    if close_after {
                        self.close_tool_tab(crate::toolkits::Tool::App(AppKind::AppDesigner));
                    }
                }
                Err(error) => {
                    self.status = error.clone();
                    self.toolkits.apps.app_designer.on_save_error(error);
                }
            }
            return;
        }
        let message = match result {
            Ok(path) => format!("Saved {}", crate::path_label::display(&path)),
            Err(error) => error,
        };
        self.status = message.clone();
        self.toolkits.apps.set_saving(kind, false, message);
    }

    fn pick_app_designer_save_path(&mut self) -> Result<PathBuf, String> {
        #[cfg(test)]
        {
            self.toolkits
                .apps
                .test_save_path
                .take()
                .ok_or_else(|| "no native save dialog available".to_string())
        }
        #[cfg(not(test))]
        {
            #[cfg(all(unix, not(target_os = "macos")))]
            let no_dbus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none_or(|v| v.is_empty());
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            let no_dbus = false;
            let default_name = format!("{}.m", self.toolkits.apps.app_designer.model.class_name);
            let picked = if no_dbus {
                None
            } else {
                rfd::FileDialog::new()
                    .add_filter("MATLAB script (*.m)", &["m"])
                    .add_filter("MatRust script (*.mr)", &["mr"])
                    .set_directory(&self.folder)
                    .set_file_name(&default_name)
                    .save_file()
            };
            picked.ok_or_else(|| {
                if no_dbus {
                    "no desktop file dialog service (D-Bus session unavailable)".to_string()
                } else {
                    "dialog was cancelled or unavailable".to_string()
                }
            })
        }
    }

    fn pick_app_script_path(&mut self, kind: AppKind) -> Option<PathBuf> {
        #[cfg(test)]
        {
            let _ = kind;
            self.toolkits.apps.test_save_path.take()
        }
        #[cfg(not(test))]
        {
            let default_name = if kind == AppKind::AppDesigner {
                format!("{}.m", self.toolkits.apps.app_designer.model.class_name)
            } else {
                kind.script_file().to_string()
            };
            rfd::FileDialog::new()
                .add_filter("MATLAB script (*.m)", &["m"])
                .add_filter("MatRust script (*.mr)", &["mr"])
                .set_directory(&self.folder)
                .set_file_name(&default_name)
                .save_file()
        }
    }
}

/// Export's desktop transaction. A return or cancellation can bypass a script's
/// catch block, so only publish the candidate after an appended completion mark.
/// Engine snapshots share immutable array storage, as Filter Designer Apply does.
pub(crate) fn execute_export(
    engine: &mut matrust::Engine,
    script: &str,
) -> matrust::Result<matrust::Execution> {
    execute_export_then(engine, script, |_| Ok(()))
}

/// Attach any captured workspace data to the candidate before publishing it.
fn execute_export_then(
    engine: &mut matrust::Engine,
    script: &str,
    finish: impl FnOnce(&mut matrust::Engine) -> matrust::Result<()>,
) -> matrust::Result<matrust::Execution> {
    let mut candidate = engine.clone();
    candidate.remove_array("mrexp_complete");
    let (body, functions) = script_sections(script);
    let output = candidate.execute(&format!("{body}\nmrexp_complete = 1;\n{functions}"))?;
    matrust::dsp::check_cancel(&engine.cancellation())?;
    if !candidate.contains_array("mrexp_complete") {
        return Err("Export stopped before completing; workspace restored".into());
    }
    candidate.remove_array("mrexp_complete");
    finish(&mut candidate)?;
    matrust::dsp::check_cancel(&engine.cancellation())?;
    *engine = candidate;
    Ok(output)
}

/// `script` wrapped so that an error restores every name in `exported` to
/// its value before the run (or clears it when it did not exist), then
/// rethrows. The error stays in `matrust_export_error`.
pub(crate) fn all_or_nothing(script: &str, exported: &[String]) -> String {
    if exported.is_empty() {
        return script.to_string();
    }
    let (script, definitions) = script_sections(script);
    let n = exported.len();
    let mut out = format!("mrexp_had = false(1, {n}); mrexp_old = cell(1, {n});\n");
    for (i, name) in exported.iter().enumerate() {
        let k = i + 1;
        out += &format!(
            "if exist('{name}', 'var') == 1, mrexp_had({k}) = true; mrexp_old{{{k}}} = {name}; end\n"
        );
    }
    out += "try\n";
    out += script.trim_end();
    out += "\ncatch matrust_export_error\n";
    for (i, name) in exported.iter().enumerate() {
        let k = i + 1;
        out += &format!(
            "if mrexp_had({k}), {name} = mrexp_old{{{k}}}; elseif exist('{name}', 'var') == 1, clear {name}; end\n"
        );
    }
    out += "clear mrexp_had mrexp_old\nrethrow(matrust_export_error)\nend\nclear mrexp_had mrexp_old\n";
    out += &definitions;
    out
}

/// Local functions belong at the script's top level, outside Export's try/catch.
/// The parser supplies byte ranges, so quoted text/comments cannot be mistaken
/// for declarations. Non-function code stays byte-for-byte unchanged.
fn script_sections(script: &str) -> (String, String) {
    let Ok(statements) = matrust::syntax::parse(script) else {
        return (script.into(), String::new());
    };
    let mut body = script.as_bytes().to_vec();
    let mut definitions = String::new();
    for statement in statements {
        if matches!(statement.kind, matrust::syntax::StatementKind::Function(_)) {
            definitions.push_str(&script[statement.span.clone()]);
            definitions.push('\n');
            for byte in &mut body[statement.span] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
        }
    }
    (
        String::from_utf8(body).expect("replacing complete parsed statements preserves UTF-8"),
        definitions,
    )
}

/// A script split before its local functions: the first line that starts
/// with `function` and everything after it (empty when there are none).
pub(crate) fn split_local_functions(script: &str) -> (&str, &str) {
    let mut start = 0;
    for line in script.split_inclusive('\n') {
        let word = line
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next();
        if word == Some("function") {
            return script.split_at(start);
        }
        start += line.len();
    }
    (script, "")
}
