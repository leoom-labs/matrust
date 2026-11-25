//! Clean-room Camera Calibrator app. Public behavior reference:
//! https://www.mathworks.com/help/vision/ref/cameracalibrator-app.html
use super::{App, AppKind, Host, Outputs, Runner, Snapshot, field};
use crate::toolkits::Action;
use eframe::egui;
use matrust::{
    Array,
    engine::camcal_app_hooks::{CalibrationValues, Launch, TransferScope},
    plot::{Axes, Figure, ImageMapping, ImageObject},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, mpsc},
};

pub(crate) type OpenSlot = Arc<Mutex<Option<Launch>>>;

/// Maximum images one folder listing may hold.
const MAX_FILES: usize = 500;
const IMAGE_FILES: [&str; 6] = ["png", "jpg", "jpeg", "tif", "tiff", "bmp"];
/// Overlays larger than this on a side are sampled for display only.
const DISPLAY_SIDE: usize = 1024;

const LIST_FUNCTION: &str = "matrust_camcal_list";
const SAMPLE_FUNCTION: &str = "matrust_camcal_sample";
const EMPTY_FOLDER: &str = "No matching images found in folder";

/// MATLAB folder listing helper reused from Image Batch Processor.
const LIST_CODE: &str = "\nfunction files = matrust_camcal_list(folder, extensions, recursive)\n\
if ~exist(folder, 'dir'), error('Camera Calibrator: input folder %s not found', folder); end\n\
files = cell(0, 1);\n\
pending = {''};\n\
while ~isempty(pending)\n\
    sub = pending{1};\n\
    pending(1) = [];\n\
    found = dir(fullfile(folder, sub));\n\
    for k = 1:numel(found)\n\
        name = found(k).name;\n\
        if found(k).isdir\n\
            if recursive && ~any(strcmp(name, {'.', '..'}))\n\
                if isempty(sub), pending{end+1} = name; else, pending{end+1} = fullfile(sub, name); end\n\
            end\n\
        else\n\
            [~, ~, ext] = fileparts(name);\n\
            if numel(ext) > 1 && any(strcmpi(ext(2:end), extensions))\n\
                if isempty(sub), files{end+1, 1} = name; else, files{end+1, 1} = fullfile(sub, name); end\n\
            end\n\
        end\n\
        if numel(files) > 500, error('Camera Calibrator: more than 500 files in folder'); end\n\
    end\n\
end\n\
if ~isempty(files), files = unique(files); end\n\
end\n";

/// Synthetic 10-image calibration dataset: 7x10 squares of 25 mm,
/// K: fx=800, fy=790, cx=322, cy=238, k1=-0.12, k2=0.05, 480x640, 2x2 supersampled.
pub const SAMPLE_TRUTH: &str =
    "Sample: 10× 480×640 boards (7×10, 25 mm), fx=800, fy=790, cx=322, cy=238, k1=-0.12, k2=0.05";

pub(crate) const SAMPLE_CODE: &str = "\nfunction folder = matrust_camcal_sample(folder)\n\
intr = cameraIntrinsics([800 790], [322 238], [480 640], 'RadialDistortion', [-0.12 0.05]);\n\
[uu, vv] = meshgrid(0.75:0.5:640.25, 0.75:0.5:480.25);\n\
und = undistortPoints([uu(:) vv(:)], intr);\n\
xn = (und(:, 1) - 322) / 800;\n\
yn = (und(:, 2) - 238) / 790;\n\
poses = [ ...\n\
    0, 0, 0, 560, 0, 0; ...\n\
    22, 8, 6, 400, -62.3, -27.0; ...\n\
    -30, 10, -8, 620, -10, 10; ...\n\
    0, 35, 3, 600, 5, 0; ...\n\
    25, -10, -6, 420, 68.4, -36.8; ...\n\
    35, 20, 12, 660, -5, -10; ...\n\
    -20, -25, 30, 640, 0, 0; ...\n\
    15, 30, -25, 600, 10, 10; ...\n\
    -30, 12, 4, 380, -43.0, 38.4; ...\n\
    5, 5, 90, 620, 0, 0 ...\n\
];\n\
centre = [100; 62.5; 0];\n\
for k = 1:size(poses, 1)\n\
    ax = poses(k, 1) * pi / 180;\n\
    ay = poses(k, 2) * pi / 180;\n\
    az = poses(k, 3) * pi / 180;\n\
    rx = [1 0 0; 0 cos(ax) -sin(ax); 0 sin(ax) cos(ax)];\n\
    ry = [cos(ay) 0 sin(ay); 0 1 0; -sin(ay) 0 cos(ay)];\n\
    rz = [cos(az) -sin(az) 0; sin(az) cos(az) 0; 0 0 1];\n\
    R = rz * rx * ry;\n\
    t = [poses(k, 5); poses(k, 6); poses(k, 4)] - R * centre;\n\
    n = R(:, 3);\n\
    denom = n(1) * xn + n(2) * yn + n(3);\n\
    s = (n' * t) ./ denom;\n\
    dx = s .* xn - t(1);\n\
    dy = s .* yn - t(2);\n\
    dz = s - t(3);\n\
    xw = dx * R(1, 1) + dy * R(2, 1) + dz * R(3, 1);\n\
    yw = dx * R(1, 2) + dy * R(2, 2) + dz * R(3, 2);\n\
    on_paper = (s > 0) & (xw >= -45) & (xw <= 245) & (yw >= -45) & (yw <= 170);\n\
    on_board = on_paper & (xw >= -25) & (xw < 225) & (yw >= -25) & (yw < 150);\n\
    c = floor((xw + 25) / 25);\n\
    r = floor((yw + 25) / 25);\n\
    black = on_board & (mod(r + c, 2) == 0);\n\
    val = 125 * ones(size(xw));\n\
    val(on_paper) = 215;\n\
    val(black) = 35;\n\
    g = reshape(val, 960, 1280);\n\
    img = uint8(round(0.25 * (g(1:2:end, 1:2:end) + g(2:2:end, 1:2:end) + g(1:2:end, 2:2:end) + g(2:2:end, 2:2:end))));\n\
    imwrite(img, fullfile(folder, sprintf('board_%02d.png', k)));\n\
end\n\
end\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Units {
    Mm,
    Cm,
    M,
    In,
}
impl Units {
    pub const ALL: [Self; 4] = [Self::Mm, Self::Cm, Self::M, Self::In];
    pub fn code(self) -> &'static str {
        match self {
            Self::Mm => "mm",
            Self::Cm => "cm",
            Self::M => "m",
            Self::In => "in",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Mm => "mm (millimeters)",
            Self::Cm => "cm (centimeters)",
            Self::M => "m (meters)",
            Self::In => "in (inches)",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "mm" | "millimeter" | "millimeters" => Some(Self::Mm),
            "cm" | "centimeter" | "centimeters" => Some(Self::Cm),
            "m" | "meter" | "meters" => Some(Self::M),
            "in" | "inch" | "inches" => Some(Self::In),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Empty,
    Folder(String),
    Files(Vec<String>),
    Sample,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageStatus {
    Pending,
    Detected,
    Rejected,
    Unused,
}
impl ImageStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Detected => "detected",
            Self::Rejected => "rejected",
            Self::Unused => "unused",
        }
    }
}

fn image_status(detected: bool, used: bool) -> ImageStatus {
    if used {
        ImageStatus::Detected
    } else if detected {
        ImageStatus::Unused
    } else {
        ImageStatus::Rejected
    }
}

#[derive(Clone, Debug)]
pub struct ImageEntry {
    pub path: String,
    pub name: String,
    pub status: ImageStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub square_size: f64,
    pub units: Units,
    pub num_radial: u8,
    pub estimate_skew: bool,
    pub estimate_tangential: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            square_size: 25.0,
            units: Units::Mm,
            num_radial: 2,
            estimate_skew: false,
            estimate_tangential: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    files: Vec<String>,
    sample_folder: Option<std::path::PathBuf>,
    pub settings: Settings,
}

#[derive(Clone, Debug)]
pub struct CalibrationResult {
    pub spec: Spec,
    values: CalibrationValues,
    pub mean_error: f64,
    pub per_view_errors: Vec<f64>,
    pub focal_length: [f64; 2],
    pub principal_point: [f64; 2],
    pub radial: Vec<f64>,
    pub tangential: [f64; 2],
    pub skew: f64,
    pub board_size: [usize; 2],
    pub image_size: [usize; 2],
    pub images_used: usize,
    pub total_images: usize,
    pub used_mask: Vec<bool>,
    pub detected_mask: Vec<bool>,
    pub detected_points: Vec<Option<Vec<[f64; 2]>>>,
    pub reprojected_points: Vec<Option<Vec<[f64; 2]>>>,
    pub resolved_files: Vec<String>,
}

#[derive(Clone)]
pub struct ImageView {
    pub axes: Axes,
    pub rows: usize,
    pub cols: usize,
    pub step: usize,
}

struct ViewRequest {
    path: String,
    undistorted: bool,
    payload: Arc<Mutex<Option<ImageView>>>,
}

/// Widget bounds recorded in tests so `click(name)` hits real controls.
#[derive(Default)]
pub struct Controls(
    #[cfg_attr(not(test), allow(dead_code))] pub BTreeMap<String, egui::Rect>,
    #[cfg(test)] pub BTreeMap<String, bool>,
);
impl Controls {
    fn note(&mut self, name: &str, response: &egui::Response) {
        #[cfg(test)]
        {
            self.0.insert(name.to_owned(), response.rect);
            self.1.insert(name.to_owned(), response.enabled());
        }
        #[cfg(not(test))]
        let _ = (name, response);
    }
}

struct SampleDir(tempfile::TempDir);
fn create_sample_dir() -> Result<Arc<SampleDir>, String> {
    tempfile::Builder::new()
        .prefix("matrust-camcal-")
        .tempdir()
        .map(|dir| Arc::new(SampleDir(dir)))
        .map_err(|e| e.to_string())
}

type ImportPayload = Arc<Mutex<Option<Result<Vec<String>, String>>>>;
type SamplePayload = Arc<Mutex<Option<(Arc<SampleDir>, ImageView)>>>;

pub struct CameraCalibrator {
    pub source: Source,
    pub images: Vec<ImageEntry>,
    pub selected_image: Option<usize>,
    pub settings: Settings,
    /// `[cameraParams, estimationErrors]`
    pub names: [String; 2],
    pub fit: bool,
    pub stale: bool,
    pub show_undistorted: bool,
    undistorted_cache: BTreeMap<String, Arc<ImageView>>,
    import_queue: std::collections::VecDeque<Source>,
    pub result: Option<CalibrationResult>,
    pub error: Option<String>,
    pub view: Option<Arc<ImageView>>,
    pub view_error: Option<String>,
    pub running: bool,
    pub calibrate_requested: bool,
    cancel_requested: bool,
    browse_files_requested: bool,
    browse_folder_requested: bool,
    serial: u64,
    delivered: Option<u64>,
    pending_spec: Option<Spec>,
    import_request: Option<Source>,
    import_append: bool,
    import_runner: Runner,
    import_payload: ImportPayload,
    import_delivered: Option<u64>,
    generation: u64,
    active_generation: u64,
    sample_dir: Option<Arc<SampleDir>>,
    sample_payload: SamplePayload,
    detection_payload: Arc<Mutex<Option<Vec<bool>>>>,
    calibration_payload: Arc<Mutex<Option<CalibrationValues>>>,
    views: Runner,
    view_delivered: Option<u64>,
    view_request: Option<ViewRequest>,
    extrinsics_runner: Runner,
    extrinsics_payload: Arc<Mutex<Option<Figure>>>,
    extrinsics_reply: Option<mpsc::Receiver<Result<(), String>>>,
    pub(crate) transfers: TransferScope,
    pub controls: Controls,
}

impl Default for CameraCalibrator {
    fn default() -> Self {
        Self {
            source: Source::Empty,
            images: Vec::new(),
            selected_image: None,
            settings: Settings::default(),
            names: ["cameraParams".into(), "estimationErrors".into()],
            fit: true,
            stale: false,
            show_undistorted: false,
            undistorted_cache: BTreeMap::new(),
            import_queue: Default::default(),
            result: None,
            error: None,
            view: None,
            view_error: None,
            running: false,
            calibrate_requested: false,
            cancel_requested: false,
            browse_files_requested: false,
            browse_folder_requested: false,
            serial: 0,
            delivered: None,
            pending_spec: None,
            import_request: None,
            import_append: false,
            import_runner: Runner::default(),
            import_payload: Default::default(),
            import_delivered: None,
            generation: 0,
            active_generation: 0,
            sample_dir: None,
            sample_payload: Default::default(),
            detection_payload: Default::default(),
            calibration_payload: Default::default(),
            views: Runner::default(),
            view_delivered: None,
            view_request: None,
            extrinsics_runner: Runner::default(),
            extrinsics_payload: Default::default(),
            extrinsics_reply: None,
            transfers: TransferScope::default(),
            controls: Controls::default(),
        }
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_owned())
}

fn list_folder_paths(engine: &mut matrust::Engine, folder: &str) -> Result<Vec<String>, String> {
    if folder.trim().is_empty() {
        return Err("Camera Calibrator: choose a folder".into());
    }
    let extensions = IMAGE_FILES
        .iter()
        .map(|ext| quote(ext))
        .collect::<Vec<_>>()
        .join(", ");
    let code = format!(
        "cc_list = {LIST_FUNCTION}({}, {{{extensions}}}, false);\n{LIST_CODE}",
        quote(folder)
    );
    engine.execute(&code)?;
    let Some(Array::Cell(cell)) = engine.array("cc_list") else {
        return Err("Camera Calibrator: folder listing did not return a file list".into());
    };
    let mut out = Vec::with_capacity(cell.len().min(MAX_FILES));
    let base = Path::new(folder);
    for idx in 0..cell.len().min(MAX_FILES) {
        if let Ok(arc) = cell.get_linear(idx)
            && let Array::Character(text) = arc.as_ref()
        {
            let rel = String::from_utf16_lossy(text.units());
            if !rel.is_empty() {
                out.push(base.join(&rel).to_string_lossy().into_owned());
            }
        }
    }
    if out.is_empty() {
        return Err(EMPTY_FOLDER.into());
    }
    Ok(out)
}

impl CameraCalibrator {
    pub fn choose(&mut self, launch: Launch) {
        match launch {
            Launch::Empty => {
                self.settings.units = Units::Mm;
            }
            Launch::Folder {
                folder,
                square_size,
                units,
            } => {
                self.set_folder(folder);
                self.settings.square_size = square_size;
                self.settings.units = units.as_deref().and_then(Units::parse).unwrap_or(Units::Mm);
            }
            Launch::Files {
                files,
                square_size,
                units,
            } => {
                self.set_files(files);
                self.settings.square_size = square_size;
                self.settings.units = units.as_deref().and_then(Units::parse).unwrap_or(Units::Mm);
            }
        }
    }

    pub fn set_folder(&mut self, folder: String) {
        self.import_append = false;
        self.clear_results();
        self.images.clear();
        self.selected_image = None;
        self.source = Source::Folder(folder.clone());
        self.import_request = Some(Source::Folder(folder));
    }

    pub fn set_files(&mut self, files: Vec<String>) {
        self.import_append = false;
        self.clear_results();
        self.images.clear();
        self.selected_image = None;
        self.source = Source::Empty;
        if !files.is_empty() {
            self.source = Source::Files(Vec::new());
            self.import_request = Some(Source::Files(files));
        }
    }

    fn append_folder(&mut self, folder: String) {
        self.queue_add(Source::Folder(folder));
    }

    pub(crate) fn append_files(&mut self, files: Vec<String>) {
        self.queue_add(Source::Files(files));
    }

    fn queue_add(&mut self, source: Source) {
        self.import_queue.push_back(source);
        self.error = None;
    }

    pub fn use_sample(&mut self) {
        self.import_append = false;
        self.clear_results();
        self.source = Source::Sample;
        self.settings.square_size = 25.0;
        self.settings.units = Units::Mm;
        self.settings.num_radial = 2;
        self.settings.estimate_skew = false;
        self.settings.estimate_tangential = false;
        self.images.clear();
        self.selected_image = None;
        self.import_request = Some(Source::Sample);
    }

    fn clear_results(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.import_runner.stop();
        self.sample_dir = None;
        self.sample_payload = Default::default();
        self.import_payload = Default::default();
        self.import_request = None;
        self.import_queue.clear();
        self.undistorted_cache.clear();
        self.stale = false;
        self.import_delivered = None;
        self.result = None;
        self.calibration_payload = Default::default();
        self.error = None;
        self.reset_view();
    }

    fn reset_view(&mut self) {
        self.view = None;
        self.view_error = None;
        self.views.stop();
        self.view_delivered = None;
        self.view_request = None;
    }

    #[cfg(test)]
    pub fn view_running(&self) -> bool {
        self.views.running() || self.import_pending()
    }

    #[cfg(test)]
    pub(crate) fn import_running(&self) -> bool {
        self.import_runner.running()
    }

    fn import_pending(&self) -> bool {
        self.import_runner.running()
            || self.import_request.is_some()
            || !self.import_queue.is_empty()
    }

    fn loading_sample(&self) -> bool {
        matches!(self.source, Source::Sample) && self.images.is_empty() && self.import_pending()
    }

    fn can_calibrate(&self) -> bool {
        self.images.len() >= 2 && !self.import_pending()
    }

    fn invalidate_result(&mut self) {
        self.stale |= self.result.take().is_some();
        self.undistorted_cache.clear();
        self.reset_view();
    }

    fn accept_import(&mut self, mut paths: Vec<String>) {
        if self.import_append {
            let mut all: Vec<String> = self.images.iter().map(|entry| entry.path.clone()).collect();
            all.append(&mut paths);
            let mut seen = std::collections::HashSet::new();
            all.retain(|path| seen.insert(path.clone()));
            if all.len() > MAX_FILES {
                self.error = Some("Camera Calibrator: more than 500 files".into());
                return;
            }
            if all.iter().eq(self.images.iter().map(|entry| &entry.path)) {
                return;
            }
            paths = all;
            self.invalidate_result();
        }
        self.images = paths
            .into_iter()
            .map(|path| ImageEntry {
                name: basename(&path),
                path,
                status: ImageStatus::Pending,
            })
            .collect();
        self.selected_image = (!self.images.is_empty()).then_some(0);
        if self.images.len() == 1 {
            self.error = Some("Camera Calibrator: need at least 2 images".into());
        }
    }

    pub(crate) fn apply_result(&mut self, result: CalibrationResult) {
        for (idx, entry) in self.images.iter_mut().enumerate() {
            entry.status = image_status(
                result.detected_mask.get(idx).copied().unwrap_or(false),
                result.used_mask.get(idx).copied().unwrap_or(false),
            );
            if let Some(resolved) = result.resolved_files.get(idx) {
                entry.path = resolved.clone();
                entry.name = basename(resolved);
            }
        }
        if self.selected_image.is_none() && !self.images.is_empty() {
            self.selected_image = Some(0);
        }
        self.result = Some(result);
        self.error = None;
        self.reset_view();
    }

    pub(crate) fn spec(&self) -> Spec {
        Spec {
            files: self.images.iter().map(|e| e.path.clone()).collect(),
            sample_folder: self.sample_dir.as_ref().map(|dir| dir.0.path().to_owned()),
            settings: self.settings.clone(),
        }
    }

    fn validate_names(&self, ws: &Snapshot) -> Result<[String; 2], String> {
        let [p, e] = &self.names;
        let [p, e] = [p.trim(), e.trim()];
        if !matrust::files::valid_name(p) || !matrust::files::valid_name(e) {
            return Err("Enter valid variable names".into());
        }
        const RESERVED: &[&str] = &[
            "break",
            "case",
            "catch",
            "classdef",
            "continue",
            "else",
            "elseif",
            "end",
            "for",
            "function",
            "global",
            "if",
            "otherwise",
            "parfor",
            "persistent",
            "return",
            "spmd",
            "switch",
            "try",
            "while",
        ];
        if [p, e].iter().any(|name| {
            RESERVED.contains(name)
                || name.starts_with("cc_")
                || name.starts_with("mrexp_")
                || *name == "matrust_camcal_private"
                || *name == SAMPLE_FUNCTION
        }) {
            return Err("Output name is reserved by the language or generated script".into());
        }
        super::check_export_name(p)?;
        super::check_export_name(e)?;
        if p == e {
            return Err("Output variable names must be distinct".into());
        }
        let _ = ws;
        Ok([p.to_owned(), e.to_owned()])
    }

    fn output(&self, ws: &Snapshot) -> Result<(&CalibrationResult, [String; 2]), String> {
        let result = self
            .result
            .as_ref()
            .ok_or("Run Calibrate before exporting or generating a script")?;
        Ok((result, self.validate_names(ws)?))
    }
}

fn build_script(
    spec: &Spec,
    names: &[String; 2],
    sample_folder: Option<&Path>,
) -> Result<String, String> {
    if !spec.settings.square_size.is_finite() || spec.settings.square_size <= 0.0 {
        return Err("Square size must be positive".into());
    }
    if spec.files.len() < 2 {
        return Err("Camera Calibrator: need at least 2 images".into());
    }
    let [params_name, errors_name] = names;
    let sq = super::number(spec.settings.square_size);
    let units = quote(spec.settings.units.code());
    let num_radial = spec.settings.num_radial;
    let skew = if spec.settings.estimate_skew {
        "true"
    } else {
        "false"
    };
    let tangential = if spec.settings.estimate_tangential {
        "true"
    } else {
        "false"
    };
    let items = spec
        .files
        .iter()
        .map(|file| {
            if sample_folder.is_some_and(|folder| Path::new(file).parent() == Some(folder)) {
                format!("fullfile(cc_folder, {})", quote(&basename(file)))
            } else {
                quote(file)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    let sample_setup = if sample_folder.is_some() {
        format!(
            "cc_folder = tempname();\n\
             mkdir(cc_folder);\n\
             cc_sample_cleanup = onCleanup(@() rmdir(cc_folder, 's'));\n\
             {SAMPLE_FUNCTION}(cc_folder);\n"
        )
    } else {
        String::new()
    };

    Ok(format!(
        "{sample_setup}cc_files = {{{items}}};\n\
         cc_info0 = imfinfo(cc_files{{1}});\n\
         cc_image_size = [cc_info0.Height, cc_info0.Width];\n\
         for cc_k = 2:numel(cc_files)\n\
             cc_ik = imfinfo(cc_files{{cc_k}});\n\
             if cc_ik.Height ~= cc_image_size(1) || cc_ik.Width ~= cc_image_size(2)\n\
                 error('Camera Calibrator: all images must have the same size');\n\
             end\n\
         end\n\
         [cc_image_points, cc_board_size, cc_images_used] = detectCheckerboardPoints(cc_files);\n\
         if sum(cc_images_used) < 2\n\
             error('Camera Calibrator: checkerboard detected in fewer than 2 images');\n\
         end\n\
         cc_world_points = generateCheckerboardPoints(cc_board_size, {sq});\n\
         [{params_name}, cc_estimator_used, {errors_name}] = estimateCameraParameters(cc_image_points, cc_world_points, ...\n\
             'ImageSize', cc_image_size, ...\n\
             'WorldUnits', {units}, ...\n\
             'NumRadialDistortionCoefficients', {num_radial}, ...\n\
             'EstimateSkew', {skew}, ...\n\
             'EstimateTangentialDistortion', {tangential});\n\
         clear cc_k cc_info0 cc_ik cc_image_size cc_world_points;\n"
    ))
}

const PRIVATE: [&str; 2] = ["cc_priv_params", "cc_priv_errors"];

fn extract_vec(out: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match out.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Calibration did not produce {name}")),
    }
}

fn extract_scalar(out: &Outputs, name: &str) -> Result<f64, String> {
    match out.get(name) {
        Some(Array::Numeric(m)) if !m.data().is_empty() => Ok(m.data()[0]),
        _ => Err(format!("Calibration did not produce {name}")),
    }
}

fn extract_points_pages(arr: Option<&Array>, used: &[bool]) -> Vec<Option<Vec<[f64; 2]>>> {
    let Some(Array::Numeric(m)) = arr else {
        return vec![None; used.len()];
    };
    let d = m.dims();
    let (rows, cols, pages) = match d.as_ref() {
        [r, c] if *c == 2 => (*r, *c, 1),
        [r, c, p] if *c == 2 => (*r, *c, *p),
        _ => return vec![None; used.len()],
    };
    let _ = cols;
    let data = m.data();
    let stride = rows * 2;
    let mut out = Vec::with_capacity(used.len());
    let mut page = 0usize;
    for &u in used {
        if u && page < pages {
            let base = page * stride;
            let pts = (0..rows)
                .filter_map(|i| {
                    let x = data[base + i];
                    let y = data[base + rows + i];
                    (x.is_finite() && y.is_finite()).then_some([x, y])
                })
                .collect();
            out.push(Some(pts));
            page += 1;
        } else {
            out.push(None);
        }
    }
    out
}

pub(crate) fn make_result(
    spec: Spec,
    out: &Outputs,
    values: CalibrationValues,
) -> Result<CalibrationResult, String> {
    let mean_error = extract_scalar(out, "cc_mean_err")?;
    let focal = extract_vec(out, "cc_focal")?;
    let pp = extract_vec(out, "cc_pp")?;
    let radial = extract_vec(out, "cc_radial")?;
    let tang = extract_vec(out, "cc_tang")?;
    let skew = extract_scalar(out, "cc_skew")?;
    let board = extract_vec(out, "cc_board_size")?;
    let imsz = extract_vec(out, "cc_imsize")?;
    let used_mask: Vec<bool> = match out.get("cc_images_used") {
        Some(Array::Logical(l)) => l.data().iter().map(|&v| v != 0).collect(),
        Some(Array::Numeric(m)) => m.data().iter().map(|&v| v != 0.0).collect(),
        _ => return Err("Calibration did not report imagesUsed".into()),
    };
    let resolved_files: Vec<String> = match out.get("cc_files") {
        Some(Array::Cell(cell)) => (0..cell.len())
            .filter_map(|i| cell.get_linear(i).ok())
            .filter_map(|item| match item.as_ref() {
                Array::Character(t) => Some(String::from_utf16_lossy(t.units())),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let detected_mask = used_mask;
    let estimator_mask: Vec<bool> = match out.get("cc_estimator_used") {
        Some(Array::Logical(l)) => l.data().iter().map(|&v| v != 0).collect(),
        Some(Array::Numeric(m)) => m.data().iter().map(|&v| v != 0.0).collect(),
        _ => return Err("Calibration did not report estimator mask".into()),
    };
    let mut i = 0;
    let used_mask: Vec<bool> = detected_mask
        .iter()
        .map(|detected| {
            if *detected {
                let used = estimator_mask.get(i).copied().unwrap_or(false);
                i += 1;
                used
            } else {
                false
            }
        })
        .collect();
    let per_view_errors = match out.get("cc_errors") {
        Some(Array::Numeric(m)) => {
            let dims = m.dims();
            let n = dims[0];
            m.data()
                .chunks_exact(2 * n)
                .map(|page| {
                    let (x, y) = page.split_at(n);
                    let (mut sum, mut points) = (0.0, 0usize);
                    for (&dx, &dy) in x.iter().zip(y) {
                        if dx.is_finite() && dy.is_finite() {
                            sum += dx.hypot(dy);
                            points += 1;
                        }
                    }
                    sum / points as f64
                })
                .collect()
        }
        _ => return Err("Calibration did not report reprojection errors".into()),
    };
    let detected_points = extract_points_pages(out.get("cc_image_points"), &detected_mask);
    let reprojected_points = extract_points_pages(out.get("cc_reprojected"), &used_mask);
    let images_used = used_mask.iter().filter(|&&u| u).count();
    let total_images = used_mask.len();
    Ok(CalibrationResult {
        spec,
        values,
        mean_error,
        per_view_errors,
        focal_length: [
            *focal.first().unwrap_or(&0.0),
            *focal.get(1).unwrap_or(&0.0),
        ],
        principal_point: [*pp.first().unwrap_or(&0.0), *pp.get(1).unwrap_or(&0.0)],
        radial,
        tangential: [*tang.first().unwrap_or(&0.0), *tang.get(1).unwrap_or(&0.0)],
        skew,
        board_size: [
            *board.first().unwrap_or(&0.0) as usize,
            *board.get(1).unwrap_or(&0.0) as usize,
        ],
        image_size: [
            *imsz.first().unwrap_or(&0.0) as usize,
            *imsz.get(1).unwrap_or(&0.0) as usize,
        ],
        images_used,
        total_images,
        used_mask,
        detected_mask,
        detected_points,
        reprojected_points,
        resolved_files,
    })
}

fn submit_calibrate(host: &mut Host<CameraCalibrator>, ctx: &egui::Context, spec: Spec) {
    let app = &mut host.app;
    app.serial += 1;
    let serial = app.serial;
    app.pending_spec = Some(spec.clone());
    app.active_generation = app.generation;
    if app.show_undistorted {
        app.reset_view();
    }
    app.result = None;
    app.stale = false;
    app.undistorted_cache.clear();
    app.error = None;
    for entry in &mut app.images {
        entry.status = ImageStatus::Pending;
    }
    let script = build_script(&spec, &PRIVATE.map(String::from), None);
    let detection_payload = app.detection_payload.clone();
    app.calibration_payload = Default::default();
    let calibration_payload = app.calibration_payload.clone();
    let sample_guard = app.sample_dir.clone();
    if let Ok(mut p) = detection_payload.lock() {
        *p = None;
    }
    host.runner.submit_task(
        ctx,
        serial,
        Box::new(move |engine| {
            let _sample_guard = sample_guard;
            if let Ok(mut detection) = detection_payload.lock() {
                *detection = None;
            }
            let script = script?;
            let marker = "if sum(cc_images_used) < 2";
            let split = script.find(marker).ok_or("Missing detection boundary")?;
            engine.execute(&script[..split])?;
            let mask = match engine.array("cc_images_used") {
                Some(Array::Logical(l)) => l.data().iter().map(|&v| v != 0).collect(),
                Some(Array::Numeric(m)) => m.data().iter().map(|&v| v != 0.0).collect(),
                _ => Vec::new(),
            };
            *detection_payload
                .lock()
                .map_err(|_| "Detection lock failed")? = Some(mask);
            engine.execute(&script[split..])?;
            matrust::dsp::check_cancel(&engine.cancellation())?;
            engine.execute(
                "cc_mean_err = cc_priv_params.MeanReprojectionError;\n\
                 cc_focal = cc_priv_params.FocalLength;\n\
                 cc_pp = cc_priv_params.PrincipalPoint;\n\
                 cc_radial = cc_priv_params.RadialDistortion;\n\
                 cc_tang = cc_priv_params.TangentialDistortion;\n\
                 cc_skew = cc_priv_params.Skew;\n\
                 cc_imsize = cc_priv_params.ImageSize;\n\
                 cc_reprojected = cc_priv_params.ReprojectedPoints;\n\
                 cc_errors = cc_priv_params.ReprojectionErrors;\n",
            )?;
            let mut out = Outputs::new();
            for name in [
                "cc_mean_err",
                "cc_focal",
                "cc_pp",
                "cc_radial",
                "cc_tang",
                "cc_skew",
                "cc_imsize",
                "cc_board_size",
                "cc_images_used",
                "cc_estimator_used",
                "cc_image_points",
                "cc_reprojected",
                "cc_errors",
                "cc_files",
            ] {
                if let Some(arr) = engine.array(name) {
                    out.insert(name.into(), arr);
                }
            }
            let values = CalibrationValues::capture(engine, PRIVATE)?;
            *calibration_payload
                .lock()
                .map_err(|_| "Calibration lock failed")? = Some(values);
            Ok(out)
        }),
    );
}

fn view_key(path: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

fn submit_view(app: &mut CameraCalibrator, ctx: &egui::Context, key: u64, path: String) {
    let values = if app.show_undistorted {
        app.result.as_ref().map(|r| r.values.clone())
    } else {
        None
    };
    let payload = Arc::new(Mutex::new(None));
    let stager = app.transfers.stager();
    // Runner delivers only the current key's result, so this describes that task.
    app.view_request = Some(ViewRequest {
        path: path.clone(),
        undistorted: values.is_some(),
        payload: payload.clone(),
    });
    let code = format!(
        "cc_v_im = imread({});\n\
         [cc_v_r, cc_v_c, ~] = size(cc_v_im);\n\
         cc_v_step = max(1, ceil(max(cc_v_r, cc_v_c) / {DISPLAY_SIDE}));\n\
         cc_v_show = cc_v_im(1:cc_v_step:end, 1:cc_v_step:end, :);\n",
        quote(&path)
    );
    app.views.submit_task(
        ctx,
        key,
        Box::new(move |engine| {
            if let Some(values) = values {
                let _hook = matrust::engine::camcal_app_hooks::present(|_| Ok(()));
                let transfer = stager.stage_export(&values, ["cc_v_params", "cc_v_errors"])?;
                engine.execute(&transfer)?;
                engine.execute(&format!(
                    "cc_v_input = imread({}); cc_v_im = undistortImage(cc_v_input, cc_v_params);",
                    quote(&path)
                ))?;
                engine
                    .execute(&code.replace(&format!("cc_v_im = imread({});", quote(&path)), ""))?;
            } else {
                engine.execute(&code)?;
            }
            let Some(Array::Numeric(m)) = engine.array("cc_v_step") else {
                return Err("View did not produce step".into());
            };
            let step = m.data()[0] as usize;
            let Some(Array::Numeric(r)) = engine.array("cc_v_r") else {
                return Err("View did not produce rows".into());
            };
            let Some(Array::Numeric(c)) = engine.array("cc_v_c") else {
                return Err("View did not produce cols".into());
            };
            let full_rows = r.data()[0] as usize;
            let full_cols = c.data()[0] as usize;
            let Some(show_arr) = engine.array("cc_v_show") else {
                return Err("View did not produce image".into());
            };
            let view = image_view(&show_arr, full_rows, full_cols, step)?;
            *payload.lock().map_err(|_| "View lock failed")? = Some(view);
            Ok(Outputs::new())
        }),
    );
}

pub(crate) fn submit_extrinsics(app: &mut CameraCalibrator, ctx: &egui::Context, view: &str) {
    let Some(result) = &app.result else {
        return;
    };
    let values = result.values.clone();
    let stager = app.transfers.stager();
    let code = format!("figure; showExtrinsics(cc_show_params, {});", quote(view));
    let payload = app.extrinsics_payload.clone();
    app.error = None;
    app.extrinsics_runner.submit_task(
        ctx,
        0,
        Box::new(move |engine| {
            let _hook = matrust::engine::camcal_app_hooks::present(|_| Ok(()));
            let transfer = stager.stage_export(&values, ["cc_show_params", "cc_show_errors"])?;
            engine.execute(&transfer)?;
            engine.execute(&code)?;
            let (_, figure) = engine
                .figures()
                .next()
                .ok_or("Extrinsics figure is missing")?;
            *payload.lock().map_err(|_| "Extrinsics lock failed")? = Some(figure.clone());
            Ok(Outputs::new())
        }),
    );
}

fn collect_extrinsics_reply(app: &mut CameraCalibrator) {
    if let Some(reply) = &app.extrinsics_reply {
        let result = match reply.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "Camera Calibrator: export ticket was not found or was already used".into(),
            )),
        };
        if let Some(result) = result {
            app.extrinsics_reply = None;
            if let Err(error) = result {
                app.error = Some(error);
            }
        }
    }
}

fn finish_extrinsics(
    app: &mut CameraCalibrator,
    ctx: &egui::Context,
    busy: bool,
    action: &mut Option<Action>,
) {
    app.extrinsics_runner.poll(ctx);
    let Some(result) = app.extrinsics_runner.result() else {
        return;
    };
    let figure = match result {
        Err(error) => Err(error.clone()),
        Ok(_) if busy || action.is_some() || ctx.will_discard() => return,
        Ok(_) => app
            .extrinsics_payload
            .lock()
            .ok()
            .and_then(|mut payload| payload.take())
            .ok_or_else(|| "Extrinsics figure is missing".into()),
    };
    app.extrinsics_runner.stop();
    match figure.and_then(|figure| app.transfers.stager().stage_figure(figure)) {
        Ok((script, reply)) => {
            app.extrinsics_reply = Some(reply);
            *action = Some(Action::Script(script));
        }
        Err(error) => {
            app.error = Some(error);
            ctx.request_repaint();
        }
    }
}

fn image_view(
    arr: &Array,
    full_rows: usize,
    full_cols: usize,
    step: usize,
) -> Result<ImageView, String> {
    let dims = arr.dims();
    let (rows, cols, channels) = match dims.as_ref() {
        [r, c] => (*r, *c, 1),
        [r, c, 1] => (*r, *c, 1),
        [r, c, 3] => (*r, *c, 3),
        _ => return Err("Unsupported image dimensions for preview".into()),
    };
    let (data, full_scale) = match arr {
        Array::Numeric(m) => (m.data().to_vec(), 1.0),
        Array::Logical(l) => (l.data().iter().map(|&v| v as f64).collect(), 1.0),
        Array::Typed(t) => {
            use matrust::typed_array::TypedData;
            let vals: Vec<f64> = match t.data() {
                TypedData::Uint8(v) => v.iter().map(|&x| x as f64).collect(),
                TypedData::Uint16(v) => v.iter().map(|&x| x as f64).collect(),
                TypedData::Int16(v) => v.iter().map(|&x| x as f64).collect(),
                TypedData::Single(v) => v.iter().map(|&x| x as f64).collect(),
                _ => return Err("Unsupported image pixel type for preview".into()),
            };
            let full_scale = match t.data() {
                TypedData::Uint16(_) => 65535.0,
                TypedData::Single(_) => 1.0,
                _ => 255.0,
            };
            (vals, full_scale)
        }
        _ => return Err("Unsupported image array type for preview".into()),
    };
    let mapping = if channels == 3 && !matches!(arr, Array::Logical(_)) {
        ImageMapping::TrueColor { full_scale }
    } else {
        ImageMapping::Scaled
    };
    let mut img = ImageObject::new(rows, cols, data.into(), mapping);
    img.x = [1.0, (1 + (cols - 1) * step) as f64];
    img.y = [1.0, (1 + (rows - 1) * step) as f64];
    img.validate()?;
    let mut axes = Axes::default();
    axes.extras.y_reversed = true;
    if mapping == ImageMapping::Scaled {
        axes.extras.clim = Some([0.0, full_scale]);
    }
    axes.extras.images.push(img);
    axes.extras.colormap = Some(Arc::new((0..256).map(|i| [i as f64 / 255.; 3]).collect()));
    Ok(ImageView {
        axes,
        rows: full_rows,
        cols: full_cols,
        step,
    })
}

pub(crate) fn refresh(host: &mut Host<CameraCalibrator>, ctx: &egui::Context, _ws: &Snapshot) {
    if host.app.active_generation != host.app.generation {
        host.runner.stop();
        host.app.pending_spec = None;
        host.app.active_generation = host.app.generation;
    }
    if !host.app.import_runner.running()
        && host.app.import_request.is_none()
        && host.app.import_delivered == host.app.import_runner.key()
        && let Some(next) = host.app.import_queue.pop_front()
    {
        host.app.import_append = true;
        host.app.import_request = Some(next);
        host.app.generation = host.app.generation.wrapping_add(1);
    }
    if let Some(request) = host.app.import_request.take() {
        let payload = host.app.import_payload.clone();
        let sample_payload = host.app.sample_payload.clone();
        let key = host.app.generation;
        host.app.import_runner.submit_task(
            ctx,
            key,
            Box::new(move |engine| {
                if let Ok(mut prior) = payload.lock() {
                    *prior = None;
                }
                if let Ok(mut sample) = sample_payload.lock() {
                    *sample = None;
                }
                let result = match request {
                    Source::Folder(folder) => list_folder_paths(engine, &folder),
                    Source::Files(files) => {
                        let mut seen = std::collections::HashSet::new();
                        let files: Vec<_> = files
                            .into_iter()
                            .filter(|p| {
                                Path::new(p)
                                    .extension()
                                    .and_then(|ext| ext.to_str())
                                    .is_some_and(|ext| {
                                        IMAGE_FILES
                                            .iter()
                                            .any(|supported| ext.eq_ignore_ascii_case(supported))
                                    })
                            })
                            .filter(|p| seen.insert(p.clone()))
                            .collect();
                        if files.len() > MAX_FILES {
                            Err("Camera Calibrator: more than 500 files".into())
                        } else {
                            Ok(files)
                        }
                    }
                    Source::Sample => {
                        let dir = create_sample_dir()?;
                        let folder = dir.0.path().to_string_lossy().into_owned();
                        engine.execute(&format!(
                            "cc_sample_folder = {SAMPLE_FUNCTION}({});\n{SAMPLE_CODE}",
                            quote(&folder)
                        ))?;
                        matrust::dsp::check_cancel(&engine.cancellation())?;
                        let paths = (1..=10)
                            .map(|k| {
                                dir.0
                                    .path()
                                    .join(format!("board_{k:02}.png"))
                                    .to_string_lossy()
                                    .into_owned()
                            })
                            .collect();
                        let first = dir.0.path().join("board_01.png");
                        engine.execute(&format!(
                            "cc_sample_preview = imread({});",
                            quote(&first.to_string_lossy())
                        ))?;
                        let preview = engine
                            .array("cc_sample_preview")
                            .ok_or("Sample preview missing")?;
                        let view = image_view(&preview, 480, 640, 1)?;
                        matrust::dsp::check_cancel(&engine.cancellation())?;
                        *sample_payload.lock().map_err(|_| "Sample lock failed")? =
                            Some((dir, view));
                        Ok(paths)
                    }
                    _ => unreachable!(),
                };
                *payload.lock().map_err(|_| "Import lock failed")? = Some(result);
                Ok(Outputs::new())
            }),
        );
    } else {
        host.app.import_runner.poll(ctx);
    }
    if host.app.import_delivered != host.app.import_runner.key()
        && host.app.import_runner.result().is_some()
    {
        host.app.import_delivered = host.app.import_runner.key();
        if let Some((sample, view)) = host
            .app
            .sample_payload
            .lock()
            .ok()
            .and_then(|mut p| p.take())
        {
            let path = sample
                .0
                .path()
                .join("board_01.png")
                .to_string_lossy()
                .into_owned();
            host.app.sample_dir = Some(sample);
            host.app.view = Some(Arc::new(view));
            host.app.view_delivered = Some(view_key(&path));
        }
        let result = host
            .app
            .import_payload
            .lock()
            .ok()
            .and_then(|mut p| p.take())
            .or_else(|| {
                host.app
                    .import_runner
                    .result()
                    .and_then(|r| r.as_ref().err().cloned())
                    .map(Err)
            });
        if let Some(result) = result {
            match result {
                Ok(paths) => host.app.accept_import(paths),
                Err(error) => {
                    if !host.app.import_append {
                        host.app.images.clear();
                        host.app.selected_image = None;
                    }
                    host.app.error = Some(error);
                }
            }
        }
    }
    host.runner.poll(ctx);
    if host.app.delivered != host.runner.key()
        && let Some(result) = host.runner.result()
    {
        host.app.delivered = host.runner.key();
        let pending = host.app.pending_spec.take();
        match result {
            Ok(out) => {
                if let Some(spec) = pending {
                    let values = host
                        .app
                        .calibration_payload
                        .lock()
                        .ok()
                        .and_then(|mut p| p.take());
                    let result = values
                        .ok_or_else(|| "Calibration did not retain the fitted objects".into())
                        .and_then(|values| make_result(spec, out, values));
                    match result {
                        Ok(res) => host.app.apply_result(res),
                        Err(e) => {
                            host.app.error = Some(e);
                        }
                    }
                }
            }
            Err(e) => {
                if let Some(mask) = host
                    .app
                    .detection_payload
                    .lock()
                    .ok()
                    .and_then(|mut p| p.take())
                {
                    for (entry, detected) in host.app.images.iter_mut().zip(mask) {
                        entry.status = if detected {
                            ImageStatus::Detected
                        } else {
                            ImageStatus::Rejected
                        };
                    }
                }
                host.app.error = Some(e.clone());
            }
        }
    }

    if std::mem::take(&mut host.app.calibrate_requested) && !host.runner.running() {
        let spec = host.app.spec();
        submit_calibrate(host, ctx, spec);
    }

    let app = &mut host.app;
    app.views.poll(ctx);
    if app.view_delivered != app.views.key()
        && let Some(result) = app.views.result()
        && let Some(request) = app.view_request.take()
    {
        app.view_delivered = app.views.key();
        match result {
            Ok(_) => {
                app.view = request
                    .payload
                    .lock()
                    .ok()
                    .and_then(|mut v| v.take())
                    .map(Arc::new);
                if request.undistorted
                    && let Some(view) = app.view.clone()
                {
                    // Keep previews bounded even for the 500-image limit.
                    if app.undistorted_cache.len() >= 4
                        && !app.undistorted_cache.contains_key(&request.path)
                        && let Some(old) = app.undistorted_cache.keys().next().cloned()
                    {
                        app.undistorted_cache.remove(&old);
                    }
                    app.undistorted_cache.insert(request.path, view);
                }
                app.view_error = None;
            }
            Err(e) => {
                app.view = None;
                app.view_error = Some(e.clone());
            }
        }
    }

    let selected_path = app
        .selected_image
        .and_then(|idx| app.images.get(idx))
        .map(|entry| entry.path.clone());
    match selected_path {
        Some(path) => {
            let undistort = app.show_undistorted && app.result.is_some();
            let key = view_key(&path) ^ if undistort { u64::MAX } else { 0 };
            if undistort && let Some(view) = app.undistorted_cache.get(&path) {
                app.view = Some(view.clone());
                app.view_delivered = Some(key);
            }
            if app.view_delivered != Some(key)
                && !app.views.running()
                && app.views.key() != Some(key)
            {
                submit_view(app, ctx, key, path);
            }
        }
        None => app.reset_view(),
    }
    app.running = host.runner.running();
}

pub(crate) fn stop(host: &mut Host<CameraCalibrator>) {
    host.runner.stop();
    host.app.delivered = None;
    host.app.pending_spec = None;
    host.app.calibration_payload = Default::default();
    host.app.running = false;
    host.app.calibrate_requested = false;
    host.message = "Calibration cancelled".into();
}

pub(crate) fn close(host: &mut Host<CameraCalibrator>) {
    stop(host);
    host.message.clear();
    host.app.reset_view();
    host.app.import_runner.stop();
    host.app.import_delivered = None;
    host.app.extrinsics_runner.stop();
    host.app.extrinsics_payload = Default::default();
    host.app.extrinsics_reply = None;
    host.app.transfers = TransferScope::default();
    if host.app.sample_dir.is_some() || matches!(host.app.source, Source::Sample) {
        host.app.set_files(Vec::new());
    }
}

impl App for CameraCalibrator {
    fn kind(&self) -> AppKind {
        AppKind::CameraCalibrator
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let (res, names) = self.output(ws)?;
        let sample_folder = res.spec.sample_folder.as_deref();
        let body = build_script(&res.spec, &PRIVATE.map(String::from), sample_folder)?;
        let helpers = if sample_folder.is_some() {
            SAMPLE_CODE
        } else {
            ""
        };
        let params = &names[0];
        let mut plots = format!("figure; showReprojectionErrors({params});\n");
        if res.images_used <= 63 {
            plots.push_str(&format!(
                "figure; showExtrinsics({params});\nfigure; showExtrinsics({params}, 'PatternCentric');\n"
            ));
        } else {
            plots.push_str(&format!(
                "% Extrinsics plots skipped: {} used views exceed the 64-series figure limit\n",
                res.images_used
            ));
        }
        Ok(format!(
            "[{}, {}] = matrust_camcal_private();\n{plots}function [{}, {}] = matrust_camcal_private()\n{body}\nend\n{helpers}",
            names[0], names[1], PRIVATE[0], PRIVATE[1]
        ))
    }
    fn inputs(&self) -> Vec<String> {
        Vec::new()
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (String::new(), Vec::new())
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _ws: &Snapshot) {
        if self.loading_sample() {
            ui.disable();
        }
        ui.horizontal_wrapped(|ui| {
            let r = ui.add(egui::Button::new("Add images"));
            self.controls.note("Add images", &r);
            if r.clicked() {
                self.browse_files_requested = true;
            }
            let r = ui.add(egui::Button::new("Add folder"));
            self.controls.note("Add folder", &r);
            if r.clicked() {
                self.browse_folder_requested = true;
            }
            let r = ui.add(egui::Button::new("Use sample data"));
            self.controls.note("Use sample data", &r);
            if r.clicked() {
                self.use_sample();
            }
        });
        let remove = ui.add_enabled(
            self.selected_image.is_some() && !self.running && !self.import_pending(),
            egui::Button::new("Remove"),
        );
        self.controls.note("Remove", &remove);
        if remove.clicked()
            && let Some(i) = self.selected_image
        {
            self.images.remove(i);
            self.selected_image = if self.images.is_empty() {
                None
            } else {
                Some(i.min(self.images.len() - 1))
            };
            self.invalidate_result();
        }
        let r = ui.button("Clear");
        self.controls.note("Clear", &r);
        if r.clicked() {
            self.set_files(Vec::new());
        }
        if matches!(self.source, Source::Sample) {
            ui.add_space(crate::tokens::XS);
            crate::tokens::helper(ui, SAMPLE_TRUTH);
        }
        ui.add_space(crate::tokens::S);

        let p = crate::tokens::palette(ui.ctx());
        ui.label(
            egui::RichText::new(format!("IMAGES ({})", self.images.len()))
                .font(crate::panels::caption_semibold(ui))
                .color(p.text_muted),
        );
        ui.add_space(crate::tokens::XS);
        if self.loading_sample() {
            ui.horizontal(|ui| {
                ui.spinner();
                crate::tokens::helper(ui, "Loading sample images…");
            });
        } else if self.images.is_empty() {
            crate::tokens::helper(
                ui,
                "No images loaded. Add images, a folder, or Use sample data.",
            );
        } else {
            let mut selection = None;
            egui::ScrollArea::vertical()
                .id_salt("camcal_images_scroll")
                .max_height(200.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (idx, entry) in self.images.iter().enumerate() {
                        let selected = self.selected_image == Some(idx);
                        let label = format!("{}  —  {}", entry.name, entry.status.label());
                        let r = ui.add(
                            egui::Button::selectable(selected, label)
                                .min_size(egui::vec2(ui.available_width(), 24.0)),
                        );
                        self.controls.note(&format!("image:{idx}"), &r);
                        if r.clicked() {
                            selection = Some(idx);
                        }
                    }
                });
            if let Some(idx) = selection {
                self.reset_view();
                self.selected_image = Some(idx);
            }
        }

        ui.add_space(crate::tokens::M);
        field(ui, "Checkerboard square size", |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.settings.square_size)
                        .speed(0.5)
                        .max_decimals(4),
                );
                let r = egui::ComboBox::from_id_salt("camcal_units")
                    .width(ui.available_width())
                    .selected_text(self.settings.units.code())
                    .show_ui(ui, |ui| {
                        for u in Units::ALL {
                            let r = ui.selectable_value(&mut self.settings.units, u, u.label());
                            self.controls.note(&format!("units:{}", u.code()), &r);
                        }
                    });
                self.controls.note("units", &r.response);
            });
        });

        ui.add_space(crate::tokens::S);
        field(ui, "Radial distortion", |ui| {
            ui.horizontal(|ui| {
                let r2 = crate::tokens::toggle(ui, self.settings.num_radial == 2, "2 coefficients");
                self.controls.note("radial:2", &r2);
                if r2.clicked() {
                    self.settings.num_radial = 2;
                }
                let r3 = crate::tokens::toggle(ui, self.settings.num_radial == 3, "3 coefficients");
                self.controls.note("radial:3", &r3);
                if r3.clicked() {
                    self.settings.num_radial = 3;
                }
            });
        });

        let r_skew = ui.checkbox(&mut self.settings.estimate_skew, "Estimate skew");
        self.controls.note("Skew", &r_skew);
        let r_tang = ui.checkbox(
            &mut self.settings.estimate_tangential,
            "Estimate tangential distortion",
        );
        self.controls.note("Tangential", &r_tang);

        ui.add_space(crate::tokens::S);
        ui.horizontal_wrapped(|ui| {
            let valid_square =
                self.settings.square_size.is_finite() && self.settings.square_size > 0.0;
            let can = !self.running && self.can_calibrate() && valid_square;
            let r = ui.add_enabled(can, egui::Button::new("Calibrate"));
            self.controls.note("Calibrate", &r);
            if r.clicked() {
                self.calibrate_requested = true;
            }
            let r = ui.add_enabled(self.running, egui::Button::new("Cancel"));
            self.controls.note("Cancel", &r);
            if r.clicked() {
                self.cancel_requested = true;
            }
        });
        if !self.settings.square_size.is_finite() || self.settings.square_size <= 0.0 {
            super::expression_message(ui, Err("Square size must be positive and finite".into()));
        }
        if self.running {
            crate::tokens::helper(ui, "Calibrating… detecting checkerboards and optimizing");
        }
        if let Some(error) = &self.error {
            super::expression_message(ui, Err(error.clone()));
        }
    }

    fn main_caption(&self) -> &'static str {
        "CHECKERBOARD VIEW"
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        if self.loading_sample() {
            ui.disable();
        }
        ui.horizontal_wrapped(|ui| {
            let r = crate::tokens::toggle(ui, self.fit, "Fit");
            self.controls.note("Fit", &r);
            if r.clicked() {
                self.fit = true;
            }
            let r = crate::tokens::toggle(ui, !self.fit, "100%");
            self.controls.note("100%", &r);
            if r.clicked() {
                self.fit = false;
            }
        });
        let toggle = ui.add_enabled(
            self.result.is_some(),
            egui::Checkbox::new(&mut self.show_undistorted, "Show Undistorted"),
        );
        self.controls.note("Show Undistorted", &toggle);
        if toggle.changed() {
            self.reset_view();
        }
        ui.add_space(crate::tokens::S);
        if let Some(error) = &self.view_error {
            ui.colored_label(crate::tokens::palette(ui.ctx()).error, error);
        }
        let width = ui.available_width().max(64.0);
        let height = (ui.available_height() - 32.0).max(200.0);
        if let Some(view) = &self.view {
            let (h, w) = (view.rows as f32, view.cols as f32);
            let scale = if self.fit {
                (width / w).min(height / h).max(0.001)
            } else {
                1.0
            };
            let size = egui::vec2(w * scale, h * scale);
            let selected_idx = self.selected_image;
            let detected = (!self.show_undistorted)
                .then_some(selected_idx)
                .flatten()
                .and_then(|i| self.result.as_ref()?.detected_points.get(i))
                .and_then(|o| o.as_ref())
                .cloned();
            let reprojected = (!self.show_undistorted)
                .then_some(selected_idx)
                .flatten()
                .and_then(|i| self.result.as_ref()?.reprojected_points.get(i))
                .and_then(|o| o.as_ref())
                .cloned();

            let marker_palette = crate::tokens::palette(ui.ctx());
            egui::ScrollArea::both()
                .id_salt("camcal_view_scroll")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let id = egui::Id::new("camcal_view_image");
                    egui_plot::Plot::new(id)
                        .width(size.x)
                        .height(size.y)
                        .invert_y(true)
                        .show_axes([false, false])
                        .show_grid([false, false])
                        .show_background(false)
                        .allow_zoom(false)
                        .allow_scroll(false)
                        .allow_drag(false)
                        .allow_boxed_zoom(false)
                        .allow_double_click_reset(false)
                        .show(ui, |plot| {
                            plot.set_plot_bounds(egui_plot::PlotBounds::from_min_max(
                                [0.5, 0.5],
                                [w as f64 + 0.5, h as f64 + 0.5],
                            ));
                            crate::figure_images::add(plot, &view.axes, id);
                            if let Some(pts) = detected {
                                plot.points(
                                    egui_plot::Points::new("Detected", pts)
                                        .shape(egui_plot::MarkerShape::Circle)
                                        .radius(4.0)
                                        .filled(false)
                                        .color(marker_palette.success),
                                );
                            }
                            if let Some(pts) = reprojected {
                                plot.points(
                                    egui_plot::Points::new("Reprojected", pts)
                                        .shape(egui_plot::MarkerShape::Plus)
                                        .radius(4.5)
                                        .color(marker_palette.accent),
                                );
                            }
                        });
                });
            let img_name = self
                .selected_image
                .and_then(|i| self.images.get(i))
                .map(|e| e.name.as_str())
                .unwrap_or("");
            let sampled = if view.step > 1 {
                format!("    display sampled 1:{}", view.step)
            } else {
                String::new()
            };
            crate::tokens::helper(
                ui,
                if self.show_undistorted && self.result.is_some() {
                    format!("{img_name}    Undistorted{sampled}")
                } else {
                    format!("{img_name}    Detected (o)    Reprojected (+){sampled}")
                },
            );
        } else {
            let msg = if self.loading_sample() {
                "Loading sample images…"
            } else if self.images.is_empty() {
                "Add images or Use sample data to begin"
            } else if self.running {
                "Calibrating…"
            } else {
                "Press Calibrate to detect checkerboards and estimate camera parameters"
            };
            ui.allocate_ui(egui::vec2(width, height), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(msg);
                });
            });
        }
    }

    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(res) = &self.result else {
            return vec![(
                "Result",
                if self.stale {
                    "Images changed — Calibrate again"
                } else {
                    "None yet"
                }
                .into(),
            )];
        };
        let mut rows = vec![
            ("Mean error", format!("{} px", super::short(res.mean_error))),
            (
                "Focal length",
                format!(
                    "[{}, {}]",
                    super::short(res.focal_length[0]),
                    super::short(res.focal_length[1])
                ),
            ),
            (
                "Principal point",
                format!(
                    "[{}, {}]",
                    super::short(res.principal_point[0]),
                    super::short(res.principal_point[1])
                ),
            ),
            (
                "Images used",
                format!(
                    "{} / {} ({}×{})",
                    res.images_used, res.total_images, res.image_size[0], res.image_size[1]
                ),
            ),
            (
                "Board",
                format!(
                    "{}×{} ({} {})",
                    res.board_size[0],
                    res.board_size[1],
                    super::short(res.spec.settings.square_size),
                    res.spec.settings.units.code()
                ),
            ),
            (
                "Radial",
                format!(
                    "[{}]",
                    res.radial
                        .iter()
                        .map(|&v| super::short(v))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        ];
        if res.spec.settings.estimate_tangential {
            rows.push((
                "Tangential",
                format!(
                    "[{}, {}]",
                    super::short(res.tangential[0]),
                    super::short(res.tangential[1])
                ),
            ));
        }
        if res.spec.settings.estimate_skew {
            rows.push(("Skew", super::short(res.skew)));
        }
        rows
    }

    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let labels = ["Camera parameters", "Estimation errors"];
        for (label, name) in labels.iter().zip(self.names.iter_mut()) {
            super::name_field(ui, label, name, ws);
        }
    }

    fn exported(&self, _: &Snapshot) -> Vec<String> {
        self.names.to_vec()
    }

    fn summary(&self) -> Option<String> {
        self.result.as_ref().map(|r| {
            format!(
                "Camera Calibrator: {} images, mean error {} px",
                r.images_used,
                super::short(r.mean_error)
            )
        })
    }

    fn export_label(&self) -> &'static str {
        "Export camera parameters"
    }
}

pub(crate) fn show(
    host: &mut Host<CameraCalibrator>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    collect_extrinsics_reply(&mut host.app);
    refresh(host, ui.ctx(), ws);
    let mut export = false;
    let mut generate = false;
    let mut extrinsics = None;
    super::three_areas(
        ui,
        AppKind::CameraCalibrator,
        "CHECKERBOARD VIEW",
        |ui, area| match area {
            super::Area::Inputs => host.app.inputs_ui(ui, ws),
            super::Area::Main => host.app.main_ui(ui),
            super::Area::Output => {
                if host.app.loading_sample() {
                    ui.disable();
                }
                super::result_block(ui, &host.app.results());
                ui.add_space(crate::tokens::S);
                ui.label("Reprojection Errors");
                if let Some(res) = &host.app.result {
                    let palette = crate::tokens::palette(ui.ctx());
                    let used: Vec<usize> = res
                        .used_mask
                        .iter()
                        .enumerate()
                        .filter_map(|(i, used)| used.then_some(i))
                        .collect();
                    let bars: Vec<_> = res
                        .per_view_errors
                        .iter()
                        .enumerate()
                        .map(|(i, value)| {
                            egui_plot::Bar::new((i + 1) as f64, *value).width(0.8).fill(
                                if used.get(i).copied() == host.app.selected_image {
                                    palette.accent
                                } else {
                                    palette.text_muted
                                },
                            )
                        })
                        .collect();
                    let mut selected = None;
                    egui::ScrollArea::horizontal()
                        .id_salt("camcal_errors_scroll")
                        .show(ui, |ui| {
                            egui_plot::Plot::new("camcal_errors")
                                .width(ui.available_width().max(bars.len() as f32 * 25.0))
                                .height(180.0)
                                .allow_drag(false)
                                .allow_zoom(false)
                                .allow_scroll(false)
                                .show(ui, |plot| {
                                    #[cfg(test)]
                                    for (i, value) in res.per_view_errors.iter().enumerate() {
                                        let pos = plot.screen_from_plot(egui_plot::PlotPoint::new(
                                            (i + 1) as f64,
                                            value / 2.0,
                                        ));
                                        host.app.controls.0.insert(
                                            format!("bar:{i}"),
                                            egui::Rect::from_center_size(pos, egui::vec2(2.0, 2.0)),
                                        );
                                    }
                                    plot.bar_chart(egui_plot::BarChart::new("Mean error", bars));
                                    plot.hline(
                                        egui_plot::HLine::new("Overall mean", res.mean_error)
                                            .color(palette.accent),
                                    );
                                    if plot.response().clicked()
                                        && let Some(pos) = plot.pointer_coordinate()
                                    {
                                        let i = pos.x.round() as isize - 1;
                                        if i >= 0
                                            && (pos.x - (i + 1) as f64).abs() <= 0.4
                                            && pos.y >= 0.0
                                            && res
                                                .per_view_errors
                                                .get(i as usize)
                                                .is_some_and(|value| pos.y <= *value)
                                        {
                                            selected = used.get(i as usize).copied();
                                        }
                                    }
                                });
                        });
                    if let Some(i) = selected {
                        host.app.selected_image = Some(i);
                        host.app.reset_view();
                    }
                } else {
                    crate::tokens::helper(ui, "Calibrate to see errors");
                }
                ui.horizontal_wrapped(|ui| {
                    let ready = host.app.result.is_some()
                        && !busy
                        && host.app.extrinsics_runner.key().is_none();
                    let r = ui.add_enabled(ready, egui::Button::new("Show extrinsics"));
                    host.app.controls.note("Show extrinsics", &r);
                    if r.clicked() {
                        extrinsics = Some("CameraCentric");
                    }
                    let r = ui.add_enabled(ready, egui::Button::new("PatternCentric"));
                    host.app.controls.note("PatternCentric", &r);
                    if r.clicked() {
                        extrinsics = Some("PatternCentric");
                    }
                });
                ui.add_space(crate::tokens::M);
                host.app.names_ui(ui, ws);
                ui.add_space(crate::tokens::S);
                let result = host.app.output(ws).map(|_| ());
                let ready = !busy && !host.saving && !host.app.running && result.is_ok();
                let wide = egui::vec2(ui.available_width(), crate::tokens::CONTROL);
                let r = ui.add_enabled(ready, egui::Button::new("Export").min_size(wide));
                host.app.controls.note("Export", &r);
                export = r.clicked();
                let r = ui.add_enabled(ready, egui::Button::new("Generate script").min_size(wide));
                host.app.controls.note("Generate script", &r);
                generate = r.clicked();
                if let Err(error) = result {
                    crate::tokens::helper(ui, error);
                }
                if !host.message.is_empty() {
                    ui.add_space(crate::tokens::S);
                    crate::tokens::helper(ui, &host.message);
                }
            }
        },
    );
    if let Some(view) = extrinsics {
        submit_extrinsics(&mut host.app, ui.ctx(), view);
    }
    if export && let Ok((result, names)) = host.app.output(ws) {
        let stager = host.app.transfers.stager();
        match stager.stage_export(&result.values, names.each_ref().map(String::as_str)) {
            Ok(script) => {
                host.app.error = None;
                host.message = format!("Exported {} to the workspace", names.join(", "));
                *action = Some(Action::Script(script));
            }
            Err(error) => {
                host.message.clear();
                host.app.error = Some(error);
            }
        }
    }
    if generate {
        *action = host.generate(ws, super::now());
    }
    if std::mem::take(&mut host.app.cancel_requested) {
        stop(host);
    }
    if !ui.ctx().will_discard()
        && std::mem::take(&mut host.app.browse_files_requested)
        && let Some(files) = pick_files(ws)
    {
        host.app.append_files(files);
    }
    if !ui.ctx().will_discard()
        && std::mem::take(&mut host.app.browse_folder_requested)
        && let Some(folder) = pick_folder(ws)
    {
        host.app.append_folder(folder);
    }
    refresh(host, ui.ctx(), ws);
    finish_extrinsics(&mut host.app, ui.ctx(), busy, action);
    host.script_error = None;
    host.run_error = host
        .app
        .error
        .clone()
        .or_else(|| host.app.view_error.clone());
}

fn pick_files(ws: &Snapshot) -> Option<Vec<String>> {
    #[cfg(test)]
    {
        let _ = ws;
        None
    }
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Images", &IMAGE_FILES)
            .add_filter("All files", &["*"]);
        if let Some(folder) = ws.folder() {
            dialog = dialog.set_directory(folder);
        }
        dialog.pick_files().map(|paths| {
            paths
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
    }
}

fn pick_folder(ws: &Snapshot) -> Option<String> {
    #[cfg(test)]
    {
        let _ = ws;
        None
    }
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(folder) = ws.folder() {
            dialog = dialog.set_directory(folder);
        }
        dialog
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    }
}

pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .camera_calibrator
        .lock()
        .ok()
        .and_then(|mut s| s.take());
    if let Some(launch) = launch
        && !workspace
            .worker
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        workspace.toolkits.apps.camera_calibrator.app.choose(launch);
        workspace.open_app(crate::toolkits::Tool::App(AppKind::CameraCalibrator));
    }
}
