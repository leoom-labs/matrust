//! Image Segmenter: a list of mask steps on a gray image. One function
//! builds the script (`[BW, maskedImage] = segmentImage(X)` and its call);
//! the private run adds read-only probes that fetch a display-sized copy of
//! the image and mask, the region count and the foreground area. Rust only
//! validates, draws and maps clicks to pixel coordinates.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, unused_name};
use eframe::egui;
use matrust::Array;
use std::hash::{Hash, Hasher};

pub const MAX_STEPS: usize = 32;
/// Longest side drawn; larger images are subsampled for display only.
pub const MAX_DISPLAY: usize = 2048;
const MAX_HISTORY: usize = 64;
const MAX_ITERATIONS: f64 = 10_000.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Otsu,
    Manual,
    Adaptive,
    KMeans,
    FloodFill,
    ActiveContour,
    Watershed,
    FillHoles,
    ClearBorder,
    RemoveSmall,
    Morphology,
    Invert,
}
impl StepKind {
    pub const ALL: [Self; 12] = [
        Self::Otsu,
        Self::Manual,
        Self::Adaptive,
        Self::KMeans,
        Self::FloodFill,
        Self::ActiveContour,
        Self::Watershed,
        Self::FillHoles,
        Self::ClearBorder,
        Self::RemoveSmall,
        Self::Morphology,
        Self::Invert,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Otsu => "Threshold: global Otsu",
            Self::Manual => "Threshold: manual level",
            Self::Adaptive => "Threshold: adaptive",
            Self::KMeans => "k-means clusters",
            Self::FloodFill => "Flood fill from a point",
            Self::ActiveContour => "Active contour",
            Self::Watershed => "Watershed split",
            Self::FillHoles => "Fill holes",
            Self::ClearBorder => "Clear border objects",
            Self::RemoveSmall => "Remove small objects",
            Self::Morphology => "Morphology",
            Self::Invert => "Invert mask",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphOp {
    Erode,
    Dilate,
    Open,
    Close,
}
impl MorphOp {
    pub const ALL: [Self; 4] = [Self::Erode, Self::Dilate, Self::Open, Self::Close];
    pub fn function(self) -> &'static str {
        match self {
            Self::Erode => "imerode",
            Self::Dilate => "imdilate",
            Self::Open => "imopen",
            Self::Close => "imclose",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Erode => "erode",
            Self::Dilate => "dilate",
            Self::Open => "open",
            Self::Close => "close",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Disk,
    Square,
    Line,
}
impl Shape {
    pub const ALL: [Self; 3] = [Self::Disk, Self::Square, Self::Line];
    fn label(self) -> &'static str {
        match self {
            Self::Disk => "disk",
            Self::Square => "square",
            Self::Line => "line",
        }
    }
    fn size_label(self) -> &'static str {
        match self {
            Self::Disk => "Radius (1–100 px)",
            Self::Square => "Width (1–201 px)",
            Self::Line => "Length (1–201 px)",
        }
    }
}

/// One mask step. Every field keeps its text so edits never lose input;
/// `call` validates them.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: StepKind,
    /// Manual level, 0–1 of the image range.
    pub level: String,
    pub sensitivity: String,
    /// Adaptive: foreground darker than its surroundings.
    pub dark: bool,
    pub k: String,
    /// Which k-means cluster is the foreground: 1 = brightest.
    pub cluster: String,
    /// Flood fill seed: 1-based row and column of the full image.
    pub point: Option<[usize; 2]>,
    pub tolerance: String,
    pub iterations: String,
    /// Active contour: edge-based instead of Chan-Vese.
    pub edge: bool,
    pub pixels: String,
    pub op: MorphOp,
    pub shape: Shape,
    pub size: String,
    pub angle: String,
}
impl Step {
    pub fn new(kind: StepKind) -> Self {
        Self {
            kind,
            level: "0.5".into(),
            sensitivity: "0.5".into(),
            dark: false,
            k: "2".into(),
            cluster: "1".into(),
            point: None,
            tolerance: "0.1".into(),
            iterations: "100".into(),
            edge: false,
            pixels: "50".into(),
            op: MorphOp::Open,
            shape: Shape::Disk,
            size: "2".into(),
            angle: "0".into(),
        }
    }
    /// The step's line in the list.
    pub fn summary(&self) -> String {
        match self.kind {
            StepKind::Otsu => "Otsu threshold".into(),
            StepKind::Manual => format!("Threshold > {}", self.level.trim()),
            StepKind::Adaptive => format!(
                "Adaptive {}, sensitivity {}",
                if self.dark { "dark" } else { "bright" },
                self.sensitivity.trim()
            ),
            StepKind::KMeans => format!(
                "k-means k={}, cluster {}",
                self.k.trim(),
                self.cluster.trim()
            ),
            StepKind::FloodFill => match self.point {
                Some([r, c]) => format!("Flood fill ({r}, {c}) ±{}", self.tolerance.trim()),
                None => "Flood fill (click the image)".into(),
            },
            StepKind::ActiveContour => format!(
                "Active contour {} × {}",
                if self.edge { "edge" } else { "Chan-Vese" },
                self.iterations.trim()
            ),
            StepKind::Watershed => "Watershed split".into(),
            StepKind::FillHoles => "Fill holes".into(),
            StepKind::ClearBorder => "Clear border".into(),
            StepKind::RemoveSmall => format!("Remove objects < {} px", self.pixels.trim()),
            StepKind::Morphology => format!(
                "{} {} {}",
                self.op.label(),
                self.shape.label(),
                self.size.trim()
            ),
            StepKind::Invert => "Invert".into(),
        }
    }
    /// The step's MATLAB lines inside the generated function (gray image
    /// `I` as double in [0, 1], mask `BW`).
    pub fn code(&self) -> Result<String, String> {
        Ok(match self.kind {
            StepKind::Otsu => "BW = imbinarize(I);\n".into(),
            StepKind::Manual => {
                let level = scalar(&self.level, "Level", 0., 1.)?;
                format!("BW = imbinarize(I, {});\n", number(level))
            }
            StepKind::Adaptive => {
                let s = scalar(&self.sensitivity, "Sensitivity", 0., 1.)?;
                format!(
                    "BW = imbinarize(I, 'adaptive', 'Sensitivity', {}, 'ForegroundPolarity', '{}');\n",
                    number(s),
                    if self.dark { "dark" } else { "bright" }
                )
            }
            StepKind::KMeans => {
                let k = integer(&self.k, "Clusters k", 2., 16.)?;
                let j = integer(&self.cluster, "Cluster", 1., k)?;
                format!(
                    "rng(0); % k-means starts are random; a fixed seed makes the result repeatable\n\
                     L = imsegkmeans(im2single(I), {k});\n\
                     m = zeros(1, {k});\n\
                     for c = 1:{k}, m(c) = mean(I(L == c)); end\n\
                     [~, order] = sort(m, 'descend'); % cluster 1 = brightest\n\
                     BW = L == order({j});\n",
                    k = number(k),
                    j = number(j)
                )
            }
            StepKind::FloodFill => {
                let [r, c] = self
                    .point
                    .ok_or("Flood fill: click the image to choose its seed point")?;
                let tolerance = scalar(&self.tolerance, "Tolerance", 0., 1.)?;
                format!(
                    "BW = BW | grayconnected(I, {r}, {c}, {});\n",
                    number(tolerance)
                )
            }
            StepKind::ActiveContour => {
                let n = integer(&self.iterations, "Iterations", 1., MAX_ITERATIONS)?;
                format!(
                    "BW = activecontour(I, BW, {}, '{}');\n",
                    number(n),
                    if self.edge { "edge" } else { "Chan-Vese" }
                )
            }
            StepKind::Watershed => {
                "D = -bwdist(~BW);\nD(~BW) = Inf;\nBW = BW & watershed(D) > 0;\n".into()
            }
            StepKind::FillHoles => "BW = imfill(BW, 'holes');\n".into(),
            StepKind::ClearBorder => "BW = imclearborder(BW);\n".into(),
            StepKind::RemoveSmall => {
                let p = integer(&self.pixels, "Smallest object (px)", 0., 1e9)?;
                format!("BW = bwareaopen(BW, {});\n", number(p))
            }
            StepKind::Morphology => {
                let element = match self.shape {
                    Shape::Disk => format!(
                        "strel('disk', {}, 0)",
                        number(integer(&self.size, "Radius", 1., 100.)?)
                    ),
                    Shape::Square => format!(
                        "strel('square', {})",
                        number(integer(&self.size, "Width", 1., 201.)?)
                    ),
                    Shape::Line => format!(
                        "strel('line', {}, {})",
                        number(integer(&self.size, "Length", 1., 201.)?),
                        number(scalar(&self.angle, "Angle", -360., 360.)?)
                    ),
                };
                format!("BW = {}(BW, {element});\n", self.op.function())
            }
            StepKind::Invert => "BW = ~BW;\n".into(),
        })
    }
}

/// A numeric field: one expression for the parser, then one finite literal
/// in [lo, hi].
fn scalar(text: &str, what: &str, lo: f64, hi: f64) -> Result<f64, String> {
    if text.len() > 64 {
        return Err(format!("{what}: at most 64 characters"));
    }
    super::single_expression(text).map_err(|e| format!("{what}: {e}"))?;
    let v = text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what}: enter one finite numeric literal"))?;
    if !(lo..=hi).contains(&v) {
        return Err(format!(
            "{what} must be from {} to {}",
            number(lo),
            number(hi)
        ));
    }
    Ok(v)
}
fn integer(text: &str, what: &str, lo: f64, hi: f64) -> Result<f64, String> {
    let v = scalar(text, what, lo, hi)?;
    if v.fract() != 0. {
        return Err(format!("{what} must be a whole number"));
    }
    Ok(v)
}
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A workspace variable the app can segment: a real uint8, uint16, single,
/// double or logical gray (M×N) or RGB (M×N×3) image.
pub fn is_image(a: &Array) -> bool {
    let dims = a.dims();
    matches!(a, Array::Numeric(_) | Array::Typed(_) | Array::Logical(_))
        && matches!(
            a.class(),
            "uint8" | "uint16" | "single" | "double" | "logical"
        )
        && !a.is_complex()
        && dims.iter().all(|d| *d > 0)
        && (dims.len() == 2 || dims.len() == 3 && dims[2] == 3)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Workspace,
    File,
}

/// What the main view draws, from the last good run.
#[derive(Clone, Debug, Default)]
pub struct View {
    /// Full image size and channels of the source.
    pub rows: usize,
    pub cols: usize,
    pub channels: usize,
    pub regions: usize,
    pub area: usize,
    /// Display copy (subsampled by `stride`), column-major.
    pub display: [usize; 2],
    pub stride: usize,
    pub gray: Vec<u8>,
    pub mask: Vec<bool>,
    /// Changes with every delivered run (texture cache key).
    pub generation: u64,
}

pub struct ImageSegmenter {
    pub source: SourceKind,
    pub variable: String,
    pub file: String,
    pub steps: Vec<Step>,
    pub selected: Option<usize>,
    /// Earlier step lists, newest last.
    history: Vec<(Vec<Step>, Option<usize>)>,
    /// The step and field the newest history entry was taken for, so typing
    /// in one field is one undo step.
    coalesce: Option<(usize, &'static str)>,
    pub mask_name: String,
    pub masked_name: String,
    pub opacity: f32,
    pub mask_only: bool,
    pub view: Option<View>,
    /// The host's runner state, once per frame (Stop shows while it runs).
    pub running: bool,
    /// Stop was clicked for these steps: no run until they change or Run.
    stopped: Option<(Vec<Step>, String, String)>,
    texture: Option<(u64, egui::TextureHandle)>,
    generation: u64,
}
impl Default for ImageSegmenter {
    fn default() -> Self {
        Self {
            source: SourceKind::Workspace,
            variable: String::new(),
            file: String::new(),
            steps: Vec::new(),
            selected: None,
            history: Vec::new(),
            coalesce: None,
            mask_name: "BW".into(),
            masked_name: "maskedImage".into(),
            opacity: 0.5,
            mask_only: false,
            view: None,
            running: false,
            stopped: None,
            texture: None,
            generation: 0,
        }
    }
}

struct Plan {
    script: String,
    /// The source expression the probe reads again (the variable, or imread).
    source: String,
}

impl ImageSegmenter {
    fn remember(&mut self, key: Option<(usize, &'static str)>) {
        if key.is_some() && key == self.coalesce {
            return;
        }
        self.history.push((self.steps.clone(), self.selected));
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.coalesce = key;
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    /// Restores the step list before the last change.
    pub fn undo(&mut self) {
        if let Some((steps, selected)) = self.history.pop() {
            self.steps = steps;
            self.selected = selected.filter(|k| *k < self.steps.len());
            self.coalesce = None;
        }
    }
    /// Adds a step after the selected one (else at the end) and selects it.
    pub fn add(&mut self, kind: StepKind) -> Result<(), String> {
        if self.steps.len() >= MAX_STEPS {
            return Err(format!("At most {MAX_STEPS} steps"));
        }
        self.remember(None);
        let at = self
            .selected
            .filter(|k| *k < self.steps.len())
            .map_or(self.steps.len(), |k| k + 1);
        self.steps.insert(at, Step::new(kind));
        self.selected = Some(at);
        Ok(())
    }
    pub fn move_up(&mut self, k: usize) {
        if k > 0 && k < self.steps.len() {
            self.remember(None);
            self.steps.swap(k - 1, k);
            self.selected = Some(k - 1);
        }
    }
    pub fn move_down(&mut self, k: usize) {
        if k + 1 < self.steps.len() {
            self.remember(None);
            self.steps.swap(k, k + 1);
            self.selected = Some(k + 1);
        }
    }
    pub fn remove(&mut self, k: usize) {
        if k < self.steps.len() {
            self.remember(None);
            self.steps.remove(k);
            self.selected = if self.steps.is_empty() {
                None
            } else {
                Some(k.min(self.steps.len() - 1))
            };
        }
    }
    /// A click on the image at a 1-based full-image pixel: the selected
    /// flood fill step takes it as its seed.
    pub fn click(&mut self, row: usize, col: usize) -> bool {
        let Some(k) = self.selected.filter(|k| *k < self.steps.len()) else {
            return false;
        };
        if self.steps[k].kind != StepKind::FloodFill || self.steps[k].point == Some([row, col]) {
            return false;
        }
        self.remember(None);
        self.steps[k].point = Some([row, col]);
        true
    }
    /// Stop: the running computation is cancelled; nothing runs until the
    /// steps change or Run.
    pub fn stop(&mut self) {
        self.stopped = Some((self.steps.clone(), self.variable.clone(), self.file.clone()));
    }
    pub fn run(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }
    fn source_expression(&self, ws: &Snapshot) -> Result<String, String> {
        match self.source {
            SourceKind::Workspace => {
                if self.variable.is_empty() {
                    return Err("Image Segmenter: choose a workspace image".into());
                }
                super::single_expression(&self.variable)?;
                if self.variable.len() > 63 || !matrust::files::valid_name(&self.variable) {
                    return Err("Image Segmenter: choose a workspace image".into());
                }
                let a = ws.get(&self.variable).ok_or_else(|| {
                    format!("Workspace variable {} does not exist", self.variable)
                })?;
                if !is_image(a) {
                    return Err(format!(
                        "Image Segmenter: {} is not a real uint8, uint16, single, double or logical gray or RGB image",
                        self.variable
                    ));
                }
                Ok(self.variable.clone())
            }
            SourceKind::File => {
                let path = self.file.trim();
                if path.is_empty() {
                    return Err("Image Segmenter: choose an image file".into());
                }
                if path.len() > 1024 || path.contains(['\n', '\r']) {
                    return Err(
                        "Image Segmenter: the file path is too long or has a line break".into(),
                    );
                }
                let expression = format!("imread({})", quote(path));
                super::single_expression(&expression)?;
                Ok(expression)
            }
        }
    }
    fn check_names(&self, ws: &Snapshot) -> Result<(), String> {
        for name in [&self.mask_name, &self.masked_name] {
            if name.len() > 63 || !matrust::files::valid_name(name) {
                return Err(format!(
                    "Image Segmenter: {name} is not a valid variable name (at most 63 characters)"
                ));
            }
            super::single_expression(name)?;
            super::check_export_name(name)?;
            if self.source == SourceKind::Workspace && *name == self.variable {
                return Err(format!(
                    "Image Segmenter: {name} is the source image; choose another output name"
                ));
            }
            if name.starts_with("is_probe_") {
                return Err(format!("Image Segmenter: {name} is reserved by the app"));
            }
        }
        if self.mask_name == self.masked_name {
            return Err("Image Segmenter: the mask and masked image need different names".into());
        }
        let _ = ws;
        Ok(())
    }
    /// The function's name: segmentImage unless a workspace variable or an
    /// output would shadow it.
    fn function_name(&self, ws: &Snapshot) -> String {
        unused_name("segmentImage", |n| {
            ws.contains(n) || n == self.mask_name || n == self.masked_name || n == self.variable
        })
    }
    /// The generated function alone (its name is `segmentImage` in a clean
    /// workspace).
    pub fn function(&self, ws: &Snapshot) -> Result<String, String> {
        let name = self.function_name(ws);
        let mut f = format!(
            "function [BW, maskedImage] = {name}(X)\n\
             % Replays the Image Segmenter steps on a gray or RGB image X. BW is the\n\
             % logical mask; maskedImage is X with the pixels outside BW set to 0.\n\
             if ~ismember(class(X), {{'uint8', 'uint16', 'single', 'double', 'logical'}}) || ~(ismatrix(X) || (ndims(X) == 3 && size(X, 3) == 3))\n    \
                 error('{name}: X must be a uint8, uint16, single, double or logical gray (M-by-N) or RGB (M-by-N-by-3) image');\n\
             end\n\
             if size(X, 3) == 3\n    I = rgb2gray(X);\nelse\n    I = X;\nend\n\
             I = im2double(I);\n\
             BW = false(size(I));\n"
        );
        for (k, step) in self.steps.iter().enumerate() {
            let code = step.code().map_err(|e| {
                format!(
                    "Image Segmenter: step {} ({}): {e}",
                    k + 1,
                    step.kind.label()
                )
            })?;
            f.push_str(&format!("% {}. {}\n{code}", k + 1, step.kind.label()));
        }
        f.push_str("maskedImage = X;\nmaskedImage(repmat(~BW, [1 1 size(X, 3)])) = 0;\nend\n");
        Ok(f)
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.steps.len() > MAX_STEPS {
            return Err(format!("Image Segmenter: at most {MAX_STEPS} steps"));
        }
        let source = self.source_expression(ws)?;
        self.check_names(ws)?;
        let function = self.function(ws)?;
        let name = self.function_name(ws);
        let script = format!(
            "% Image Segmenter: {} step(s) on {}.\n[{}, {}] = {name}({source});\n\n{function}",
            self.steps.len(),
            match self.source {
                SourceKind::Workspace => self.variable.clone(),
                SourceKind::File => self.file.trim().to_owned(),
            },
            self.mask_name,
            self.masked_name,
        );
        Ok(Plan { script, source })
    }
}

const PROBES: [&str; 3] = ["is_probe_gray", "is_probe_mask", "is_probe_info"];

impl App for ImageSegmenter {
    fn kind(&self) -> AppKind {
        AppKind::ImageSegmenter
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.stopped.is_some() {
            return Err(
                "Image Segmenter: stopped. Press Run or change a step to compute again.".into(),
            );
        }
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        match self.source {
            SourceKind::Workspace if !self.variable.is_empty() => vec![self.variable.clone()],
            _ => Vec::new(),
        }
    }
    fn prelude(&self, ws: &Snapshot) -> String {
        // Relative image paths resolve as in the workspace.
        match ws.folder() {
            Some(folder) if self.source == SourceKind::File => {
                format!("try\ncd({});\nend\n", quote(&folder.to_string_lossy()))
            }
            _ => String::new(),
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let bw = &self.mask_name;
        let code = format!(
            "is_probe_stride = max(1, ceil(max(size({bw}, 1), size({bw}, 2)) / {MAX_DISPLAY}));\n\
             is_probe_x = {src};\n\
             is_probe_channels = size(is_probe_x, 3);\n\
             is_probe_x = is_probe_x(1:is_probe_stride:end, 1:is_probe_stride:end, :);\n\
             if size(is_probe_x, 3) == 3, is_probe_x = rgb2gray(is_probe_x); end\n\
             is_probe_gray = im2uint8(is_probe_x);\n\
             is_probe_mask = {bw}(1:is_probe_stride:end, 1:is_probe_stride:end);\n\
             is_probe_cc = bwconncomp({bw}, 8);\n\
             is_probe_info = [size({bw}, 1), size({bw}, 2), is_probe_channels, is_probe_cc.NumObjects, nnz({bw}), is_probe_stride];\n\
             clear is_probe_x is_probe_cc\n",
            src = plan.source,
        );
        (code, PROBES.iter().map(|s| (*s).to_owned()).collect())
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let info = match outputs.get("is_probe_info") {
            Some(Array::Numeric(m)) if m.data().len() == 6 => m.data().to_vec(),
            _ => return Err("Image Segmenter: missing image size".into()),
        };
        let gray = match outputs.get("is_probe_gray") {
            Some(Array::Typed(t)) if t.class() == matrust::NumericClass::Uint8 => t,
            _ => return Err("Image Segmenter: missing display image".into()),
        };
        let mask = match outputs.get("is_probe_mask") {
            Some(Array::Logical(m)) => m,
            _ => return Err("Image Segmenter: the mask is not logical".into()),
        };
        let display = [gray.rows(), gray.cols()];
        if mask.rows() != display[0] || mask.cols() != display[1] || gray.dims().len() != 2 {
            return Err("Image Segmenter: display image and mask sizes differ".into());
        }
        let n = display[0] * display[1];
        let gray: Vec<u8> = (0..n).map(|i| gray.real(i) as u8).collect();
        let mask: Vec<bool> = mask.data().iter().map(|v| *v != 0).collect();
        self.generation += 1;
        self.view = Some(View {
            rows: info[0] as usize,
            cols: info[1] as usize,
            channels: info[2] as usize,
            regions: info[3] as usize,
            area: info[4] as usize,
            display,
            stride: info[5] as usize,
            gray,
            mask,
            generation: self.generation,
        });
        Ok(())
    }
    fn prepare(&mut self, _ws: &Snapshot) {
        if let Some((steps, variable, file)) = &self.stopped
            && (*steps != self.steps || *variable != self.variable || *file != self.file)
        {
            self.stopped = None;
        }
        self.selected = self.selected.filter(|k| *k < self.steps.len());
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.source_ui(ui, ws);
        ui.add_space(crate::tokens::S);
        self.steps_ui(ui);
        if let Some(k) = self.selected.filter(|k| *k < self.steps.len()) {
            ui.add_space(crate::tokens::S);
            self.editor_ui(ui, k);
        }
        ui.add_space(crate::tokens::S);
        ui.horizontal(|ui| {
            if self.running && !self.is_stopped() {
                if ui.button("Stop").clicked() {
                    self.stop();
                }
                crate::tokens::helper(ui, "Computing…");
            } else if self.is_stopped() && ui.button("Run").clicked() {
                self.run();
            }
        });
    }
    fn main_caption(&self) -> &'static str {
        "IMAGE AND MASK"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let Some(view) = &self.view else {
            crate::tokens::helper(
                ui,
                "Choose a workspace image or a file, then add steps to build a mask.",
            );
            return;
        };
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.mask_only, "Mask only");
            ui.add_enabled_ui(!self.mask_only, |ui| {
                ui.label("Opacity");
                ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).fixed_decimals(2));
            });
        });
        ui.add_space(crate::tokens::XS);
        if view.display[0] == 0 || view.display[1] == 0 {
            crate::tokens::helper(ui, "The image is empty");
            return;
        }
        let accent = crate::tokens::palette(ui.ctx()).accent;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        view.generation.hash(&mut hasher);
        self.opacity.to_bits().hash(&mut hasher);
        self.mask_only.hash(&mut hasher);
        accent.to_array().hash(&mut hasher);
        let key = hasher.finish();
        let texture = match &self.texture {
            Some((k, texture)) if *k == key => texture.clone(),
            _ => {
                let image = compose(view, accent, self.opacity, self.mask_only);
                let texture =
                    ui.ctx()
                        .load_texture("image-segmenter", image, egui::TextureOptions::NEAREST);
                self.texture = Some((key, texture.clone()));
                texture
            }
        };
        let [dr, dc] = view.display;
        let available = ui.available_size() - egui::vec2(0., crate::tokens::L + crate::tokens::S);
        let scale = (available.x / dc as f32)
            .min(available.y.max(160.) / dr as f32)
            .max(f32::MIN_POSITIVE);
        let size = egui::vec2(dc as f32 * scale, dr as f32 * scale);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
            egui::Color32::WHITE,
        );
        let (rows, cols, stride) = (view.rows, view.cols, view.stride);
        let flood = self
            .selected
            .and_then(|k| self.steps.get(k))
            .is_some_and(|s| s.kind == StepKind::FloodFill);
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let [row, col] = pixel(rect, pos, rows, cols);
            self.click(row, col);
        }
        let hint = if flood {
            "Click the image to place the flood fill seed."
        } else {
            "Select a flood fill step to place its seed by clicking."
        };
        let shown = if stride > 1 {
            format!(" Shown at 1/{stride} scale; steps use the full image.")
        } else {
            String::new()
        };
        crate::tokens::helper(ui, format!("{hint}{shown}"));
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(v) = &self.view else {
            return vec![("Steps", self.steps.len().to_string())];
        };
        let total = (v.rows * v.cols).max(1);
        vec![
            (
                "Image",
                format!(
                    "{} × {}{}",
                    v.rows,
                    v.cols,
                    if v.channels == 3 { " RGB → gray" } else { "" }
                ),
            ),
            ("Steps", self.steps.len().to_string()),
            ("Regions (8-connected)", v.regions.to_string()),
            (
                "Foreground area",
                format!(
                    "{} px ({}%)",
                    v.area,
                    super::short(100. * v.area as f64 / total as f64)
                ),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Mask (logical)", &mut self.mask_name, ws);
        name_field(ui, "Masked image", &mut self.masked_name, ws);
        super::expression_message(ui, self.check_names(ws));
        crate::tokens::helper(
            ui,
            "Generate script saves the function [BW, maskedImage] = segmentImage(X) and its call.",
        );
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        vec![self.mask_name.clone(), self.masked_name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.view.as_ref().map(|v| {
            format!(
                "Image Segmenter: {} region(s) · {} px foreground",
                v.regions, v.area
            )
        })
    }
}

/// The 1-based full-image pixel under `pos` in the drawn `rect`.
pub fn pixel(rect: egui::Rect, pos: egui::Pos2, rows: usize, cols: usize) -> [usize; 2] {
    let fx = ((pos.x - rect.left()) / rect.width()).clamp(0., 1.);
    let fy = ((pos.y - rect.top()) / rect.height()).clamp(0., 1.);
    let col = ((fx * cols as f32) as usize).min(cols.saturating_sub(1)) + 1;
    let row = ((fy * rows as f32) as usize).min(rows.saturating_sub(1)) + 1;
    [row, col]
}

/// The display texture: gray with the mask blended in the accent colour, or
/// the mask alone (white on black). Texture rows are image rows.
pub fn compose(
    view: &View,
    accent: egui::Color32,
    opacity: f32,
    mask_only: bool,
) -> egui::ColorImage {
    let [rows, cols] = view.display;
    let mut rgb = Vec::with_capacity(rows * cols * 3);
    let a = opacity.clamp(0., 1.);
    let [ar, ag, ab, _] = accent.to_array();
    for r in 0..rows {
        for c in 0..cols {
            let i = c * rows + r;
            let on = view.mask[i];
            if mask_only {
                let v = if on { 255 } else { 0 };
                rgb.extend([v, v, v]);
            } else if on {
                let g = view.gray[i] as f32;
                let mix = |x: u8| (g * (1. - a) + x as f32 * a).round() as u8;
                rgb.extend([mix(ar), mix(ag), mix(ab)]);
            } else {
                let g = view.gray[i];
                rgb.extend([g, g, g]);
            }
        }
    }
    egui::ColorImage::from_rgb([cols, rows], &rgb)
}

impl ImageSegmenter {
    fn source_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        field(ui, "Source", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.source, SourceKind::Workspace, "Workspace");
                ui.selectable_value(&mut self.source, SourceKind::File, "File");
            });
        });
        match self.source {
            SourceKind::Workspace => {
                field(ui, "Image variable", |ui| {
                    egui::ComboBox::from_id_salt("image_segmenter_source")
                        .width(ui.available_width())
                        .selected_text(if self.variable.is_empty() {
                            "Choose image"
                        } else {
                            &self.variable
                        })
                        .show_ui(ui, |ui| {
                            for (name, _) in ws.variables().filter(|(_, a)| is_image(a)) {
                                ui.selectable_value(&mut self.variable, name.to_owned(), name);
                            }
                        });
                });
                if let Some(a) = ws.get(&self.variable).filter(|a| is_image(a)) {
                    let dims = a.dims();
                    let rgb = dims.len() == 3;
                    crate::tokens::helper(
                        ui,
                        format!(
                            "{} × {} {}{}",
                            dims[0],
                            dims[1],
                            a.class(),
                            if rgb {
                                " RGB: the steps run on rgb2gray of it"
                            } else {
                                " gray"
                            }
                        ),
                    );
                }
            }
            SourceKind::File => {
                field(ui, "Image file (imread)", |ui| {
                    ui.horizontal(|ui| {
                        let browse = egui::vec2(72., crate::tokens::CONTROL);
                        ui.add(
                            crate::tokens::input(
                                egui::TextEdit::singleline(&mut self.file).char_limit(1024),
                            )
                            .desired_width(ui.available_width() - browse.x - crate::tokens::S),
                        );
                        if ui
                            .add(egui::Button::new("Browse…").min_size(browse))
                            .clicked()
                        {
                            self.browse(ws);
                        }
                    });
                });
                if let Some(v) = &self.view
                    && v.channels == 3
                {
                    crate::tokens::helper(ui, "RGB file: the steps run on rgb2gray of it");
                }
            }
        }
    }
    fn browse(&mut self, ws: &Snapshot) {
        #[cfg(test)]
        {
            let _ = ws;
        }
        #[cfg(not(test))]
        {
            let mut dialog = rfd::FileDialog::new().add_filter(
                "Images",
                &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif"],
            );
            if let Some(folder) = ws.folder() {
                dialog = dialog.set_directory(folder);
            }
            if let Some(path) = dialog.pick_file() {
                self.file = path.to_string_lossy().into_owned();
            }
        }
    }
    fn steps_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Steps");
        if self.steps.is_empty() {
            crate::tokens::helper(ui, "No steps yet: the mask is empty");
        }
        let mut select = None;
        for (k, step) in self.steps.iter().enumerate() {
            let text = format!("{}. {}", k + 1, step.summary());
            if ui
                .selectable_label(self.selected == Some(k), text)
                .clicked()
            {
                select = Some(k);
            }
        }
        if let Some(k) = select {
            self.selected = if self.selected == Some(k) {
                None
            } else {
                Some(k)
            };
        }
        ui.add_space(crate::tokens::XS);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(self.steps.len() < MAX_STEPS, |ui| {
                let add = ui.menu_button("Add step", |ui| {
                    for kind in StepKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            let _ = self.add(kind);
                            ui.close();
                        }
                    }
                });
                crate::focus_areas::register(
                    ui,
                    &add.response,
                    crate::focus_areas::Area::App(AppKind::ImageSegmenter),
                );
            });
            let k = self.selected.filter(|k| *k < self.steps.len());
            if ui
                .add_enabled(k.is_some_and(|k| k > 0), egui::Button::new("Up"))
                .clicked()
                && let Some(k) = k
            {
                self.move_up(k);
            }
            if ui
                .add_enabled(
                    k.is_some_and(|k| k + 1 < self.steps.len()),
                    egui::Button::new("Down"),
                )
                .clicked()
                && let Some(k) = k
            {
                self.move_down(k);
            }
            if ui
                .add_enabled(k.is_some(), egui::Button::new("Remove"))
                .clicked()
                && let Some(k) = k
            {
                self.remove(k);
            }
            if ui
                .add_enabled(self.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.undo();
            }
        });
    }
    /// The selected step's parameters. Each edited field is one undo step.
    fn editor_ui(&mut self, ui: &mut egui::Ui, k: usize) {
        let before = self.steps[k].clone();
        let mut field_key = None;
        let step = &mut self.steps[k];
        ui.label(
            egui::RichText::new(format!("Step {} · {}", k + 1, step.kind.label()))
                .font(crate::tokens::semibold(crate::tokens::BODY)),
        );
        ui.add_space(crate::tokens::XS);
        let mut text = |ui: &mut egui::Ui, label: &str, key: &'static str, value: &mut String| {
            field(ui, label, |ui| {
                let response = ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(value).char_limit(64))
                        .desired_width(ui.available_width()),
                );
                if response.changed() {
                    field_key = Some(key);
                }
            });
        };
        match step.kind {
            StepKind::Otsu => {
                crate::tokens::helper(ui, "Global Otsu level of the gray image.");
            }
            StepKind::Manual => text(ui, "Level (0–1)", "level", &mut step.level),
            StepKind::Adaptive => {
                text(
                    ui,
                    "Sensitivity (0–1)",
                    "sensitivity",
                    &mut step.sensitivity,
                );
                field(ui, "Foreground", |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut step.dark, false, "Brighter");
                        ui.selectable_value(&mut step.dark, true, "Darker");
                    });
                });
            }
            StepKind::KMeans => {
                text(ui, "Clusters k (2–16)", "k", &mut step.k);
                text(
                    ui,
                    "Keep cluster (1 = brightest)",
                    "cluster",
                    &mut step.cluster,
                );
            }
            StepKind::FloodFill => {
                let at = match step.point {
                    Some([r, c]) => format!("Seed: row {r}, column {c}"),
                    None => "Seed: click the image".into(),
                };
                crate::tokens::helper(ui, at);
                text(
                    ui,
                    "Tolerance (0–1 of the range)",
                    "tolerance",
                    &mut step.tolerance,
                );
                crate::tokens::helper(ui, "The connected region joins the current mask.");
            }
            StepKind::ActiveContour => {
                text(
                    ui,
                    "Iterations (1–10000)",
                    "iterations",
                    &mut step.iterations,
                );
                field(ui, "Method", |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut step.edge, false, "Chan-Vese");
                        ui.selectable_value(&mut step.edge, true, "Edge");
                    });
                });
                crate::tokens::helper(ui, "Evolves the current mask; it must not be empty.");
            }
            StepKind::Watershed => {
                crate::tokens::helper(
                    ui,
                    "Splits touching objects along watershed lines of the distance transform.",
                );
            }
            StepKind::FillHoles | StepKind::ClearBorder | StepKind::Invert => {}
            StepKind::RemoveSmall => {
                text(ui, "Remove objects under (px)", "pixels", &mut step.pixels)
            }
            StepKind::Morphology => {
                field(ui, "Operation", |ui| {
                    egui::ComboBox::from_id_salt(("image_segmenter_op", k))
                        .selected_text(step.op.label())
                        .show_ui(ui, |ui| {
                            for op in MorphOp::ALL {
                                ui.selectable_value(&mut step.op, op, op.label());
                            }
                        });
                });
                field(ui, "Structuring element", |ui| {
                    egui::ComboBox::from_id_salt(("image_segmenter_shape", k))
                        .selected_text(step.shape.label())
                        .show_ui(ui, |ui| {
                            for shape in Shape::ALL {
                                ui.selectable_value(&mut step.shape, shape, shape.label());
                            }
                        });
                });
                text(ui, step.shape.size_label(), "size", &mut step.size);
                if step.shape == Shape::Line {
                    text(ui, "Angle (degrees)", "angle", &mut step.angle);
                }
            }
        }
        super::expression_message(ui, step.code().map(|_| ()));
        if *step != before {
            let after = step.clone();
            self.steps[k] = before;
            self.remember(field_key.map(|key| (k, key)));
            self.steps[k] = after;
        }
    }
}
