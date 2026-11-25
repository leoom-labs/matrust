//! Clean-room intensity-based image registration. Public behavior reference:
//! https://www.mathworks.com/help/images/ref/registrationestimator-app.html
//! One script builder serves the private run, workspace export and saved
//! scripts; a second private Runner draws the overlay views.
use super::{App, AppKind, Host, Outputs, Runner, Snapshot, field};
use crate::toolkits::Action;
use eframe::egui;
use matrust::{
    Array,
    engine::regest_app_hooks::{GeoVariable, Launch, image_shape},
    plot::Axes,
};
use std::{
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) type OpenSlot = Arc<Mutex<Option<Launch>>>;
/// Overlays larger than this on a side are sampled for display only.
pub const DISPLAY_SIDE: usize = 1024;
/// Runs kept for one pair of images; the oldest goes first.
pub const MAX_RUNS: usize = 64;
const FUNCTION: &str = "matrust_registration";
const VIEW_FUNCTION: &str = "matrust_registration_view";
const SAMPLE_FUNCTION: &str = "matrust_registration_sample";
/// The Use sample data pair: two Gaussian blobs (fixed, 120x120) and the
/// same image moved by rigidtform2d(8,[6 -4]). Scripts carry this function,
/// so they rebuild the pair without workspace variables.
const SAMPLE_CODE: &str = "\nfunction [moving, fixed] = matrust_registration_sample()\n\
    [x, y] = meshgrid(1:120);\n\
    fixed = 255*exp(-((x-50).^2+(y-60).^2)/200)+150*exp(-((x-80).^2+(y-40).^2)/100);\n\
    moving = imwarp(fixed, rigidtform2d(8, [6 -4]), 'OutputView', imref2d(size(fixed)));\n\
    end\n";
/// Output names of the private run (never exported).
const PRIVATE: [&str; 4] = ["mrre_tform", "mrre_registered", "mrre_ref", "mrre_quality"];
#[cfg_attr(test, allow(dead_code))]
pub const IMAGE_FILES: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "tif", "tiff"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Technique {
    Monomodal,
    Multimodal,
}
impl Technique {
    pub fn label(self) -> &'static str {
        match self {
            Self::Monomodal => "Monomodal",
            Self::Multimodal => "Multimodal",
        }
    }
    fn config(self) -> &'static str {
        match self {
            Self::Monomodal => "monomodal",
            Self::Multimodal => "multimodal",
        }
    }
    fn detail(self) -> &'static str {
        match self {
            Self::Monomodal => "Mean squares, regular step gradient descent",
            Self::Multimodal => "Mattes mutual information, one-plus-one evolutionary",
        }
    }
    pub fn step_property(self) -> &'static str {
        match self {
            Self::Monomodal => "MaximumStepLength",
            Self::Multimodal => "InitialRadius",
        }
    }
    fn step_label(self) -> &'static str {
        match self {
            Self::Monomodal => "Maximum step length",
            Self::Multimodal => "Initial radius",
        }
    }
    pub fn default_step(self) -> f64 {
        match self {
            Self::Monomodal => 0.0625,
            Self::Multimodal => 0.00625,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformType {
    Translation,
    Rigid,
    Similarity,
    Affine,
}
impl TransformType {
    pub const ALL: [Self; 4] = [
        Self::Translation,
        Self::Rigid,
        Self::Similarity,
        Self::Affine,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Translation => "translation",
            Self::Rigid => "rigid",
            Self::Similarity => "similarity",
            Self::Affine => "affine",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Translation => "Translation",
            Self::Rigid => "Rigid",
            Self::Similarity => "Similarity",
            Self::Affine => "Affine",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overlay {
    FalseColor,
    Blend,
    Checkerboard,
    Difference,
}
impl Overlay {
    pub const ALL: [Self; 4] = [
        Self::FalseColor,
        Self::Blend,
        Self::Checkerboard,
        Self::Difference,
    ];
    pub fn method(self) -> &'static str {
        match self {
            Self::FalseColor => "falsecolor",
            Self::Blend => "blend",
            Self::Checkerboard => "checkerboard",
            Self::Difference => "diff",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::FalseColor => "False color",
            Self::Blend => "Blend",
            Self::Checkerboard => "Checkerboard",
            Self::Difference => "Difference",
        }
    }
}

/// Where an image comes from: a workspace variable, a file (absolute path)
/// or the built-in sample pair (Use sample data).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Variable(String),
    File(String),
    Sample,
}

/// One image input as the user edits it.
#[derive(Clone, Debug, Default)]
pub struct Picker {
    pub from_file: bool,
    /// Use sample data: the built-in pair; Workspace or File clears it.
    pub sample: bool,
    pub variable: String,
    pub path: String,
    /// imref2d variable; empty = imref2d(size(image)).
    pub reference: String,
}

/// Optimizer and transformation choices of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub technique: Technique,
    pub transform: TransformType,
    pub levels: usize,
    pub iterations: usize,
    /// MaximumStepLength (monomodal) or InitialRadius (multimodal).
    pub step: f64,
    /// Workspace transformation object; empty = none.
    pub initial: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            technique: Technique::Monomodal,
            transform: TransformType::Rigid,
            levels: 3,
            iterations: 100,
            step: Technique::Monomodal.default_step(),
            initial: String::new(),
        }
    }
}

/// Everything the script needs, resolved and checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub moving: Source,
    pub fixed: Source,
    pub moving_ref: Option<String>,
    pub fixed_ref: Option<String>,
    pub settings: Settings,
}
impl Spec {
    /// Workspace variables the script reads, in call order.
    pub fn variables(&self) -> Vec<String> {
        let mut out = Vec::new();
        for source in [&self.moving, &self.fixed] {
            if let Source::Variable(name) = source {
                out.push(name.clone());
            }
        }
        out.extend(self.moving_ref.iter().cloned());
        out.extend(self.fixed_ref.iter().cloned());
        if !self.settings.initial.is_empty() {
            out.push(self.settings.initial.clone());
        }
        out
    }
}

fn identifier(name: &str) -> Result<(), String> {
    if name.len() > 63 || !matrust::files::valid_name(name) {
        return Err(format!(
            "{name:?} is not a valid variable name (at most 63 characters)"
        ));
    }
    super::check_export_name(name)?;
    matrust::syntax::parse(&format!("{name} = 1;")).map_err(|e| e.to_string())?;
    Ok(())
}

/// Names the app's scripts use for themselves.
fn reserved(name: &str) -> bool {
    name.starts_with("mrre_")
        || name == FUNCTION
        || name == VIEW_FUNCTION
        || name == SAMPLE_FUNCTION
}

/// Checks the four export names: valid, distinct, not an input.
pub fn checked_names(names: &[String; 4], spec: &Spec) -> Result<(), String> {
    let inputs = spec.variables();
    for (k, name) in names.iter().enumerate() {
        identifier(name)?;
        if reserved(name) {
            return Err(format!("{name} is reserved by Registration Estimator"));
        }
        if names[..k].contains(name) {
            return Err(format!(
                "Two outputs are named {name}; choose different names"
            ));
        }
        if inputs.contains(name) {
            return Err(format!(
                "{name} is an input of the selected run; choose another output name"
            ));
        }
    }
    Ok(())
}

/// Function lines that read file images and default the spatial references.
fn load_lines(spec: &Spec, moving: bool, out: &mut String) -> Result<(), String> {
    let mut images = vec![("fixed", &spec.fixed, &spec.fixed_ref, "Rfixed")];
    if moving {
        images.insert(0, ("moving", &spec.moving, &spec.moving_ref, "Rmoving"));
    }
    if images
        .iter()
        .any(|(_, source, _, _)| **source == Source::Sample)
    {
        *out += &format!("[sampleMoving, sampleFixed] = {SAMPLE_FUNCTION}();\n");
    }
    for (name, source, _, _) in &images {
        match source {
            Source::File(path) => {
                let path = super::comparison_tool::path_literal(path)?;
                *out += &format!("{name} = imread({path});\n");
            }
            Source::Sample => {
                let pair = if *name == "moving" {
                    "sampleMoving"
                } else {
                    "sampleFixed"
                };
                *out += &format!("{name} = {pair};\n");
            }
            Source::Variable(_) => {}
        }
    }
    for (name, _, reference, r) in &images {
        if reference.is_none() {
            *out += &format!("{r} = imref2d(size({name}));\n");
        }
    }
    Ok(())
}

/// Parameters and call arguments of the generated function, in order.
fn parameters(spec: &Spec, moving: bool, initial: bool) -> (Vec<&'static str>, Vec<String>) {
    let mut params = Vec::new();
    let mut args = Vec::new();
    let mut add = |param: &'static str, arg: &str| {
        params.push(param);
        args.push(arg.to_owned());
    };
    if moving && let Source::Variable(name) = &spec.moving {
        add("moving", name);
    }
    if let Source::Variable(name) = &spec.fixed {
        add("fixed", name);
    }
    if moving && let Some(r) = &spec.moving_ref {
        add("Rmoving", r);
    }
    if let Some(r) = &spec.fixed_ref {
        add("Rfixed", r);
    }
    if initial && !spec.settings.initial.is_empty() {
        add("tformInitial", &spec.settings.initial);
    }
    (params, args)
}

fn gray_lines(name: &str, gray: &str) -> String {
    format!("{gray} = {name};\nif size({name}, 3) == 3\n    {gray} = rgb2gray({name});\nend\n")
}

/// The complete reproducing script: a call that leaves the transformation,
/// the registered image, its imref2d and the quality value under `names`,
/// and the local function that does the work (so no other variable changes).
pub fn build_script(spec: &Spec, names: &[String; 4]) -> Result<String, String> {
    for name in spec.variables() {
        identifier(&name)?;
        if reserved(&name) {
            return Err(format!("{name} is reserved by Registration Estimator"));
        }
    }
    for name in names {
        identifier(name)?;
    }
    let s = &spec.settings;
    if !(1..=8).contains(&s.levels) {
        return Err("Pyramid levels must be 1 to 8".into());
    }
    if !(1..=100_000).contains(&s.iterations) {
        return Err("Maximum iterations must be 1 to 100000".into());
    }
    if !s.step.is_finite() || s.step <= 0. {
        return Err(format!(
            "{} must be positive and finite",
            s.technique.step_label()
        ));
    }
    let (params, args) = parameters(spec, true, true);
    let (metric, optimizer) = match s.technique {
        Technique::Monomodal => ("MeanSquares", "RegularStepGradientDescent"),
        Technique::Multimodal => ("MattesMutualInformation", "OnePlusOneEvolutionary"),
    };
    let mut out = format!(
        "% Registration Estimator: {} transformation, {} technique\n\
         % ({metric} metric, {optimizer} optimizer).\n\
         [{}] = {FUNCTION}({});\n\n\
         function [tform, registered, Rregistered, quality] = {FUNCTION}({})\n",
        s.transform.name(),
        s.technique.config(),
        names.join(", "),
        args.join(", "),
        params.join(", "),
    );
    load_lines(spec, true, &mut out)?;
    out += "% RGB images are registered on their gray version.\n";
    out += &gray_lines("moving", "movingGray");
    out += &gray_lines("fixed", "fixedGray");
    out += &format!(
        "[optimizer, metric] = imregconfig('{}');\n\
         optimizer.MaximumIterations = {};\n\
         optimizer.{} = {};\n",
        s.technique.config(),
        s.iterations,
        s.technique.step_property(),
        super::number(s.step)
    );
    let initial = if s.initial.is_empty() {
        String::new()
    } else {
        ", 'InitialTransformation', tformInitial".into()
    };
    out += &format!(
        "tform = imregtform(movingGray, Rmoving, fixedGray, Rfixed, '{}', optimizer, metric, \
         'PyramidLevels', {}{initial});\n\
         registered = imwarp(moving, Rmoving, tform, 'OutputView', Rfixed);\n\
         Rregistered = Rfixed;\n\
         % Quality: normalized correlation of the registered and fixed images\n\
         % over the fixed frame (1 = identical up to brightness and contrast).\n",
        s.transform.name(),
        s.levels
    );
    out += &gray_lines("registered", "registeredGray");
    out += "a = double(registeredGray(:));\n\
            b = double(fixedGray(:));\n\
            a = a - mean(a);\n\
            b = b - mean(b);\n\
            quality = sum(a .* b) / sqrt(sum(a .^ 2) * sum(b .^ 2));\n\
            end\n";
    sample_function(spec, &mut out);
    Ok(out)
}

fn sample_function(spec: &Spec, out: &mut String) {
    if spec.moving == Source::Sample || spec.fixed == Source::Sample {
        *out += SAMPLE_CODE;
    }
}

/// Private display code: the imfuse overlay of the fixed image with the
/// moving image (identity placement in the fixed frame) or with a run's
/// registered image (`mrre_registered`), sampled to DISPLAY_SIDE.
pub fn view_script(spec: &Spec, overlay: Overlay, after: bool) -> Result<String, String> {
    let (mut params, mut args) = parameters(spec, !after, false);
    if after {
        params.push("registered");
        args.push("mrre_registered".into());
    }
    let mut out = format!(
        "[mrre_view, mrre_step] = {VIEW_FUNCTION}({});\nimshow(mrre_view);\n\n\
         function [C, s] = {VIEW_FUNCTION}({})\n",
        args.join(", "),
        params.join(", ")
    );
    load_lines(spec, !after, &mut out)?;
    if !after {
        out += "registered = imwarp(moving, Rmoving, transltform2d(0, 0), 'OutputView', Rfixed);\n";
    }
    out += &format!(
        "C = imfuse(fixed, registered, '{}');\n\
         s = max(1, ceil(max(size(C, 1), size(C, 2)) / {DISPLAY_SIDE}));\n\
         C = C(1:s:end, 1:s:end, :);\n\
         end\n",
        overlay.method()
    );
    sample_function(spec, &mut out);
    Ok(out)
}

/// One finished registration.
#[derive(Clone)]
pub struct Run {
    pub id: usize,
    pub spec: Spec,
    /// Workspace arrays (and object handles) it read, to notice changes.
    pub inputs: Vec<(String, Array)>,
    /// The objects among them, to rebuild in private engines.
    pub geo: Vec<GeoVariable>,
    pub seconds: f64,
    pub quality: f64,
    pub class: String,
    /// The transformation's A matrix, row-major.
    pub matrix: [[f64; 3]; 3],
    pub registered: Array,
}
impl Run {
    pub fn label(&self) -> String {
        format!(
            "{}  {} · {}",
            self.id,
            self.spec.settings.transform.label(),
            self.spec.settings.technique.label()
        )
    }
    pub fn detail(&self) -> String {
        format!(
            "{} s    r = {}",
            super::short(self.seconds),
            super::short(self.quality)
        )
    }
}

/// An overlay ready to draw.
pub struct View {
    pub key: u64,
    pub axes: Axes,
    pub rows: usize,
    pub cols: usize,
    /// Display sampling step (1 = every pixel).
    pub step: usize,
}

struct ViewRequest {
    key: u64,
    spec: Spec,
    inputs: Vec<(String, Array)>,
    geo: Vec<GeoVariable>,
    registered: Option<Array>,
}

struct Pending {
    id: usize,
    spec: Spec,
    inputs: Vec<(String, Array)>,
    geo: Vec<GeoVariable>,
    images: u64,
}

/// Rectangles of the controls, for the headless interaction tests.
#[derive(Default)]
pub struct Controls(#[cfg(test)] pub std::collections::BTreeMap<String, egui::Rect>);
impl Controls {
    fn note(&mut self, name: &str, response: &egui::Response) {
        #[cfg(test)]
        self.0.insert(name.to_owned(), response.rect);
        #[cfg(not(test))]
        let _ = (name, response);
    }
}

pub struct RegistrationEstimator {
    pub moving: Picker,
    pub fixed: Picker,
    pub settings: Settings,
    /// tform, registered image, its imref2d, quality.
    pub names: [String; 4],
    pub overlay: Overlay,
    pub after: bool,
    pub fit: bool,
    pub runs: Vec<Run>,
    pub selected: Option<usize>,
    /// Images key of the kept runs.
    runs_images: Option<u64>,
    pub view: Option<Arc<View>>,
    pub view_error: Option<String>,
    /// Registration error (inline and status line).
    pub error: Option<String>,
    /// Workspace imref2d and transformation objects (from the desktop worker).
    pub catalogue: Vec<GeoVariable>,
    pub running: bool,
    pub register_requested: bool,
    cancel_requested: bool,
    browse_requested: Option<bool>,
    next_id: usize,
    serial: u64,
    delivered: Option<u64>,
    pending: Option<Pending>,
    views: Runner,
    view_delivered: Option<u64>,
    view_payload: Arc<Mutex<Option<View>>>,
    pub controls: Controls,
}
impl Default for RegistrationEstimator {
    fn default() -> Self {
        Self {
            moving: Picker::default(),
            fixed: Picker::default(),
            settings: Settings::default(),
            names: [
                "tform".into(),
                "movingRegistered".into(),
                "Rregistered".into(),
                "registrationQuality".into(),
            ],
            overlay: Overlay::FalseColor,
            after: true,
            fit: true,
            runs: Vec::new(),
            selected: None,
            runs_images: None,
            view: None,
            view_error: None,
            error: None,
            catalogue: Vec::new(),
            running: false,
            register_requested: false,
            cancel_requested: false,
            browse_requested: None,
            next_id: 1,
            serial: 0,
            delivered: None,
            pending: None,
            views: Runner::default(),
            view_delivered: None,
            view_payload: Default::default(),
            controls: Controls::default(),
        }
    }
}

impl RegistrationEstimator {
    pub fn choose(&mut self, launch: Launch) {
        if let Launch::Images { moving, fixed } = launch {
            for (picker, name) in [(&mut self.moving, moving.0), (&mut self.fixed, fixed.0)] {
                *picker = Picker {
                    variable: name,
                    ..Picker::default()
                };
            }
        }
    }
    /// True while an overlay view is being computed.
    #[cfg(test)]
    pub fn view_running(&self) -> bool {
        self.views.running()
    }
    pub fn selected_run(&self) -> Option<&Run> {
        self.selected
            .and_then(|id| self.runs.iter().find(|r| r.id == id))
    }
    fn geo(&self, ws: &Snapshot, name: &str) -> Option<&GeoVariable> {
        self.catalogue
            .iter()
            .find(|g| g.name == name && ws.get(name).is_some_and(|a| a.same_snapshot(&g.handle)))
    }
    /// Use sample data: both images become the built-in pair (no workspace
    /// variable is written) and Rigid is selected; Register still runs it.
    /// The imref2d and Initial transformation choices are workspace
    /// variables, so they are cleared: the run and its script read none.
    pub fn use_sample(&mut self) {
        for picker in [&mut self.moving, &mut self.fixed] {
            picker.sample = true;
            picker.reference.clear();
        }
        self.settings.initial.clear();
        self.settings.transform = TransformType::Rigid;
        self.error = None;
    }
    fn source(picker: &Picker, ws: &Snapshot, role: &str) -> Result<Source, String> {
        if picker.sample {
            Ok(Source::Sample)
        } else if picker.from_file {
            if picker.path.trim().is_empty() {
                return Err(format!("Choose the {role} image file"));
            }
            let path = Path::new(picker.path.trim());
            let path = match ws.folder() {
                Some(folder) if path.is_relative() => folder.join(path),
                _ => path.to_owned(),
            };
            let path = path.to_str().ok_or("Paths must be Unicode")?.to_owned();
            super::comparison_tool::path_literal(&path)?;
            if !Path::new(&path).is_file() {
                return Err(format!("The {role} image file does not exist"));
            }
            Ok(Source::File(path))
        } else {
            if picker.variable.is_empty() {
                return Err(format!("Choose the {role} image"));
            }
            let array = ws
                .get(&picker.variable)
                .ok_or_else(|| format!("Workspace variable {} does not exist", picker.variable))?;
            image_shape(array).map_err(|e| {
                e.replacen(
                    "registrationEstimator: choose a",
                    &format!("The {role} image must be a"),
                    1,
                )
            })?;
            Ok(Source::Variable(picker.variable.clone()))
        }
    }
    /// The chosen images and spatial references, checked against the
    /// workspace; the settings are copied unchecked.
    pub fn draft_images(&self, ws: &Snapshot) -> Result<Spec, String> {
        let moving = Self::source(&self.moving, ws, "moving")?;
        let fixed = Self::source(&self.fixed, ws, "fixed")?;
        let mut refs = [None, None];
        for (slot, picker) in refs.iter_mut().zip([&self.moving, &self.fixed]) {
            if picker.reference.is_empty() {
                continue;
            }
            match self.geo(ws, &picker.reference) {
                Some(g) if g.is_imref() => *slot = Some(picker.reference.clone()),
                _ => return Err(format!("{} is not an imref2d variable", picker.reference)),
            }
        }
        let [moving_ref, fixed_ref] = refs;
        Ok(Spec {
            moving,
            fixed,
            moving_ref,
            fixed_ref,
            settings: self.settings.clone(),
        })
    }
    /// The current inputs, checked against the workspace.
    pub fn draft(&self, ws: &Snapshot) -> Result<Spec, String> {
        let spec = self.draft_images(ws)?;
        let initial = &spec.settings.initial;
        if !initial.is_empty() && !self.geo(ws, initial).is_some_and(GeoVariable::is_tform) {
            return Err(format!("{initial} is not a 2-D transformation variable"));
        }
        build_script(&spec, &PRIVATE.map(String::from))?;
        Ok(spec)
    }
    /// Identifies the two images (sources and their workspace versions):
    /// runs are kept while it stays the same. References, the initial
    /// transformation and the settings belong to each run.
    fn images_key(spec: &Spec, ws: &Snapshot) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for source in [&spec.moving, &spec.fixed] {
            source.hash(&mut h);
            if let Source::Variable(name) = source {
                ws.version(name).hash(&mut h);
            }
        }
        h.finish()
    }
    /// True when an image variable of the kept runs changed or went away.
    fn runs_stale(&self, ws: &Snapshot) -> bool {
        let Some(run) = self.runs.first() else {
            return false;
        };
        [&run.spec.moving, &run.spec.fixed]
            .into_iter()
            .any(|source| {
                let Source::Variable(name) = source else {
                    return false;
                };
                !run.inputs
                    .iter()
                    .any(|(n, a)| n == name && ws.get(name).is_some_and(|w| w.same_snapshot(a)))
            })
    }
    fn inputs(&self, spec: &Spec, ws: &Snapshot) -> Vec<(String, Array)> {
        spec.variables()
            .into_iter()
            .filter_map(|name| ws.get(&name).map(|a| (name, a.clone())))
            .collect()
    }
    /// The export names, checked for the selected run.
    fn output(&self, ws: &Snapshot) -> Result<(&Run, [String; 4]), String> {
        let run = self
            .selected_run()
            .ok_or("Register the images, then select a result")?;
        checked_names(&self.names, &run.spec)?;
        for (name, array) in &run.inputs {
            if !ws.get(name).is_some_and(|a| a.same_snapshot(array)) {
                return Err(format!(
                    "{name} changed since this run; register again before exporting"
                ));
            }
        }
        Ok((run, self.names.clone()))
    }
    /// What the overlay shows: the selected run's inputs (before or after
    /// registration), else the chosen images before registration.
    fn view_request(&self, ws: &Snapshot) -> Option<ViewRequest> {
        let after = self.after;
        let (spec, inputs, geo, registered) = match self.selected_run() {
            Some(run) => (
                run.spec.clone(),
                run.inputs.clone(),
                run.geo.clone(),
                after.then(|| run.registered.clone()),
            ),
            None => {
                let spec = self.draft_images(ws).ok()?;
                let inputs = self.inputs(&spec, ws);
                let geo = self
                    .catalogue
                    .iter()
                    .filter(|g| inputs.iter().any(|(n, _)| *n == g.name))
                    .cloned()
                    .collect();
                (spec, inputs, geo, None)
            }
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&spec.moving, &spec.fixed, &spec.moving_ref, &spec.fixed_ref).hash(&mut h);
        for (name, _) in &inputs {
            ws.version(name).hash(&mut h);
        }
        self.selected.hash(&mut h);
        registered.is_some().hash(&mut h);
        self.overlay.hash(&mut h);
        ws.folder().hash(&mut h);
        Some(ViewRequest {
            key: h.finish(),
            spec,
            inputs,
            geo,
            registered,
        })
    }
}

fn scalar(out: &Outputs, name: &str) -> Result<f64, String> {
    match out.get(name) {
        Some(Array::Numeric(m)) if m.data().len() == 1 => Ok(m.data()[0]),
        _ => Err(format!("The registration did not produce {name}")),
    }
}

fn install(
    engine: &mut matrust::Engine,
    inputs: &[(String, Array)],
    geo: &[GeoVariable],
) -> Result<(), String> {
    for (name, array) in inputs {
        match geo.iter().find(|g| g.name == *name) {
            Some(g) => g.install(engine)?,
            None => engine.insert_array(name, array.clone())?,
        }
    }
    Ok(())
}

fn submit_register(
    host: &mut Host<RegistrationEstimator>,
    ctx: &egui::Context,
    spec: Spec,
    ws: &Snapshot,
) {
    let app = &mut host.app;
    app.serial += 1;
    let serial = app.serial;
    let inputs = app.inputs(&spec, ws);
    let geo: Vec<GeoVariable> = app
        .catalogue
        .iter()
        .filter(|g| inputs.iter().any(|(n, _)| *n == g.name))
        .cloned()
        .collect();
    let names = PRIVATE.map(String::from);
    let script = build_script(&spec, &names);
    app.pending = Some(Pending {
        id: app.next_id,
        images: RegistrationEstimator::images_key(&spec, ws),
        spec,
        inputs: inputs.clone(),
        geo: geo.clone(),
    });
    app.next_id += 1;
    host.runner.submit_task(
        ctx,
        serial,
        Box::new(move |engine| {
            let script = script?;
            install(engine, &inputs, &geo)?;
            let start = Instant::now();
            engine.execute(&script)?;
            let seconds = start.elapsed().as_secs_f64();
            matrust::dsp::check_cancel(&engine.cancellation())?;
            engine.execute("mrre_class = class(mrre_tform); mrre_A = mrre_tform.A;")?;
            let mut out = Outputs::new();
            for name in ["mrre_registered", "mrre_quality", "mrre_class", "mrre_A"] {
                out.insert(
                    name.into(),
                    engine
                        .array(name)
                        .ok_or_else(|| format!("The registration did not produce {name}"))?,
                );
            }
            out.insert(
                "mrre_seconds".into(),
                Array::Numeric(matrust::Matrix::scalar(seconds)),
            );
            Ok(out)
        }),
    );
}

fn make_run(pending: Pending, out: &Outputs) -> Result<Run, String> {
    let Some(Array::Character(class)) = out.get("mrre_class") else {
        return Err("The registration did not report its class".into());
    };
    let class = String::from_utf16_lossy(class.units());
    let matrix = match out.get("mrre_A") {
        Some(Array::Numeric(m)) if m.data().len() == 9 => {
            let d = m.data();
            std::array::from_fn(|r| std::array::from_fn(|c| d[r + 3 * c]))
        }
        _ => return Err("The registration did not produce a 3-by-3 matrix".into()),
    };
    Ok(Run {
        id: pending.id,
        spec: pending.spec,
        inputs: pending.inputs,
        geo: pending.geo,
        seconds: scalar(out, "mrre_seconds")?,
        quality: scalar(out, "mrre_quality")?,
        class,
        matrix,
        registered: out
            .get("mrre_registered")
            .cloned()
            .ok_or("The registration did not produce the registered image")?,
    })
}

fn submit_view(
    app: &mut RegistrationEstimator,
    ctx: &egui::Context,
    request: ViewRequest,
    ws: &Snapshot,
) {
    let ViewRequest {
        key,
        spec,
        inputs,
        geo,
        registered,
    } = request;
    let after = registered.is_some();
    let script = view_script(&spec, app.overlay, after);
    // After registration only the fixed image and the registered one are read.
    let (_, args) = parameters(&spec, !after, false);
    let mut inputs: Vec<(String, Array)> = inputs
        .into_iter()
        .filter(|(name, _)| args.contains(name))
        .collect();
    if let Some(registered) = registered {
        inputs.push(("mrre_registered".into(), registered));
    }
    let payload = app.view_payload.clone();
    app.views.set_folder(ws.folder());
    app.views.submit_task(
        ctx,
        key,
        Box::new(move |engine| {
            let script = script?;
            install(engine, &inputs, &geo)?;
            engine.execute(&script)?;
            let image = engine
                .array("mrre_view")
                .ok_or("The overlay was not produced")?;
            let step = match engine.array("mrre_step") {
                Some(Array::Numeric(m)) if m.data().len() == 1 => m.data()[0].max(1.) as usize,
                _ => 1,
            };
            let axes = engine
                .figures()
                .next()
                .ok_or("The overlay did not produce an image")?
                .1
                .axes
                .clone();
            let dims = image.dims();
            *payload.lock().map_err(|_| "View delivery lock failed")? = Some(View {
                key,
                axes,
                rows: dims[0],
                cols: dims[1],
                step,
            });
            Ok(Outputs::new())
        }),
    );
}

pub(crate) fn refresh(host: &mut Host<RegistrationEstimator>, ctx: &egui::Context, ws: &Snapshot) {
    host.set_folder(ws.folder());
    host.runner.poll(ctx);
    if host.app.delivered != host.runner.key()
        && let Some(result) = host.runner.result()
    {
        host.app.delivered = host.runner.key();
        let pending = host.app.pending.take();
        match (result, pending) {
            (Ok(out), Some(pending)) => {
                let images = pending.images;
                match make_run(pending, out) {
                    Ok(run) => {
                        let app = &mut host.app;
                        if app.runs_images != Some(images) {
                            app.runs.clear();
                        }
                        app.runs_images = Some(images);
                        app.selected = Some(run.id);
                        app.runs.push(run);
                        if app.runs.len() > MAX_RUNS {
                            app.runs.remove(0);
                        }
                        app.after = true;
                        app.error = None;
                    }
                    Err(error) => host.app.error = Some(error),
                }
            }
            (Err(error), _) => host.app.error = Some(error.clone()),
            (Ok(_), None) => {}
        }
    }
    // Runs belong to one pair of images: other valid images or a changed
    // image variable start a new list. An invalid choice only shows its
    // error and keeps the runs.
    let images = host
        .app
        .draft_images(ws)
        .ok()
        .map(|s| RegistrationEstimator::images_key(&s, ws));
    if !host.app.runs.is_empty()
        && (host.app.runs_stale(ws) || images.is_some_and(|k| Some(k) != host.app.runs_images))
    {
        host.app.runs.clear();
        host.app.selected = None;
        host.app.runs_images = None;
    }
    if std::mem::take(&mut host.app.register_requested) {
        match host.app.draft(ws) {
            Ok(spec) => {
                host.app.error = None;
                host.message.clear();
                submit_register(host, ctx, spec, ws);
            }
            Err(error) => host.app.error = Some(error),
        }
    }
    // The overlay view follows the inputs, the selection and the mode.
    let app = &mut host.app;
    app.views.poll(ctx);
    if app.view_delivered != app.views.key()
        && let Some(result) = app.views.result()
    {
        app.view_delivered = app.views.key();
        match result {
            Ok(_) => {
                if let Some(view) = app.view_payload.lock().ok().and_then(|mut s| s.take()) {
                    app.view = Some(Arc::new(view));
                    app.view_error = None;
                }
            }
            Err(error) => app.view_error = Some(error.clone()),
        }
    }
    match app.view_request(ws) {
        Some(request) => {
            if app.view.as_ref().is_none_or(|v| v.key != request.key)
                && app.views.key() != Some(request.key)
            {
                submit_view(app, ctx, request, ws);
            }
        }
        None => {
            app.views.stop();
            app.view_delivered = None;
            app.view = None;
            app.view_error = None;
        }
    }
    app.running = host.runner.running();
}

/// Cancel: the running registration stops; earlier runs stay.
pub(crate) fn stop(host: &mut Host<RegistrationEstimator>) {
    host.runner.stop();
    host.app.delivered = None;
    host.app.pending = None;
    host.app.running = false;
    host.app.register_requested = false;
    host.message = "Registration cancelled; earlier results kept".into();
}

pub(crate) fn close(host: &mut Host<RegistrationEstimator>) {
    stop(host);
    host.message.clear();
    host.app.views.stop();
    host.app.view_delivered = None;
}

impl App for RegistrationEstimator {
    fn kind(&self) -> AppKind {
        AppKind::RegistrationEstimator
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let (run, names) = self.output(ws)?;
        build_script(&run.spec, &names)
    }
    fn inputs(&self) -> Vec<String> {
        self.selected_run()
            .map_or_else(Vec::new, |r| r.spec.variables())
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (String::new(), Vec::new())
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let images: Vec<String> = ws
            .variables()
            .filter(|(_, a)| image_shape(a).is_ok())
            .map(|(n, _)| n.to_owned())
            .collect();
        let live = |g: &&GeoVariable| ws.get(&g.name).is_some_and(|a| a.same_snapshot(&g.handle));
        let refs: Vec<String> = self
            .catalogue
            .iter()
            .filter(live)
            .filter(|g| g.is_imref())
            .map(|g| g.name.clone())
            .collect();
        let tforms: Vec<(String, &'static str)> = self
            .catalogue
            .iter()
            .filter(live)
            .filter(|g| g.is_tform())
            .map(|g| (g.name.clone(), g.class))
            .collect();
        for (role, title) in [("moving", "Moving image"), ("fixed", "Fixed image")] {
            let picker = if role == "moving" {
                &mut self.moving
            } else {
                &mut self.fixed
            };
            if picker_ui(ui, role, title, picker, &images, &refs, &mut self.controls) {
                self.browse_requested = Some(role == "moving");
            }
            ui.add_space(crate::tokens::S);
        }
        let s = &mut self.settings;
        field(ui, "Technique", |ui| {
            let old = s.technique;
            let r = egui::ComboBox::from_id_salt("regest_technique")
                .width(ui.available_width())
                .selected_text(s.technique.label())
                .show_ui(ui, |ui| {
                    for t in [Technique::Monomodal, Technique::Multimodal] {
                        let r = ui.selectable_value(&mut s.technique, t, t.label());
                        self.controls.note(&format!("technique:{}", t.label()), &r);
                    }
                });
            self.controls.note("technique", &r.response);
            if old != s.technique && s.step == old.default_step() {
                s.step = s.technique.default_step();
            }
            crate::tokens::helper(ui, s.technique.detail());
        });
        field(ui, "Transformation", |ui| {
            let r = egui::ComboBox::from_id_salt("regest_transform")
                .width(ui.available_width())
                .selected_text(s.transform.label())
                .show_ui(ui, |ui| {
                    for t in TransformType::ALL {
                        let r = ui.selectable_value(&mut s.transform, t, t.label());
                        self.controls.note(&format!("transform:{}", t.label()), &r);
                    }
                });
            self.controls.note("transform", &r.response);
        });
        ui.horizontal(|ui| {
            ui.label("Pyramid levels");
            ui.add(egui::DragValue::new(&mut s.levels).range(1..=8));
        });
        ui.horizontal(|ui| {
            ui.label("Maximum iterations");
            ui.add(egui::DragValue::new(&mut s.iterations).range(1..=100_000));
        });
        ui.horizontal(|ui| {
            ui.label(s.technique.step_label());
            let speed = s.step.abs().max(1e-6) * 0.01;
            ui.add(
                egui::DragValue::new(&mut s.step)
                    .speed(speed)
                    .range(1e-9..=1e6)
                    .max_decimals(6),
            );
        });
        ui.add_space(crate::tokens::S);
        field(ui, "Initial transformation", |ui| {
            let r = egui::ComboBox::from_id_salt("regest_initial")
                .width(ui.available_width())
                .selected_text(if s.initial.is_empty() {
                    "None"
                } else {
                    &s.initial
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.initial, String::new(), "None");
                    for (name, class) in &tforms {
                        let r = ui.selectable_value(
                            &mut s.initial,
                            name.clone(),
                            format!("{name} ({class})"),
                        );
                        self.controls.note(&format!("initial:{name}"), &r);
                    }
                });
            self.controls.note("initial", &r.response);
        });
        ui.horizontal_wrapped(|ui| {
            let r = ui.add_enabled(!self.running, egui::Button::new("Register"));
            self.controls.note("Register", &r);
            if r.clicked() {
                self.register_requested = true;
            }
            let r = ui.add_enabled(!self.running, egui::Button::new("Use sample data"));
            self.controls.note("Use sample data", &r);
            if r.clicked() {
                self.use_sample();
            }
            let r = ui.add_enabled(self.running, egui::Button::new("Cancel"));
            self.controls.note("Cancel", &r);
            if r.clicked() {
                self.cancel_requested = true;
            }
        });
        if self.running {
            crate::tokens::helper(ui, "Registering… Cancel keeps the earlier results");
        }
        if let Some(error) = &self.error {
            super::expression_message(ui, Err(error.clone()));
        }
        ui.add_space(crate::tokens::L);
        let p = crate::tokens::palette(ui.ctx());
        ui.label(
            egui::RichText::new("RESULTS")
                .font(crate::panels::caption_semibold(ui))
                .color(p.text_muted),
        );
        ui.add_space(crate::tokens::XS);
        if self.runs.is_empty() {
            crate::tokens::helper(ui, "Each Register press adds a run here");
        }
        let mut chosen = None;
        for run in self.runs.iter().rev() {
            let on = self.selected == Some(run.id);
            let r = ui.add(
                egui::Button::selectable(on, format!("{}\n{}", run.label(), run.detail()))
                    .min_size(egui::vec2(ui.available_width(), 40.)),
            );
            self.controls.note(&format!("Run {}", run.id), &r);
            if r.clicked() {
                chosen = Some(run.id);
            }
        }
        if let Some(id) = chosen {
            self.selected = Some(id);
            self.after = true;
        }
    }
    fn main_caption(&self) -> &'static str {
        "OVERLAY"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for mode in Overlay::ALL {
                let r = crate::tokens::toggle(ui, self.overlay == mode, mode.label());
                self.controls.note(mode.label(), &r);
                if r.clicked() {
                    self.overlay = mode;
                }
            }
        });
        ui.add_space(crate::tokens::XS);
        let has_run = self.selected_run().is_some();
        ui.horizontal_wrapped(|ui| {
            let r = crate::tokens::toggle(ui, !self.after || !has_run, "Before");
            self.controls.note("Before", &r);
            if r.clicked() {
                self.after = false;
            }
            let r = ui.add_enabled_ui(has_run, |ui| {
                crate::tokens::toggle(ui, self.after && has_run, "After")
            });
            self.controls.note("After", &r.inner);
            if r.inner.clicked() {
                self.after = true;
            }
            ui.add_space(crate::tokens::M);
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
        ui.add_space(crate::tokens::S);
        for error in [&self.error, &self.view_error].into_iter().flatten() {
            ui.colored_label(crate::tokens::palette(ui.ctx()).error, error);
        }
        let width = ui.available_width().max(64.);
        let height = (ui.available_height() - 32.).max(200.);
        if let Some(view) = &self.view {
            let (h, w) = (view.rows as f32, view.cols as f32);
            let scale = if self.fit {
                (width / w).min(height / h).max(0.001)
            } else {
                view.step as f32
            };
            let size = egui::vec2(w * scale, h * scale);
            egui::ScrollArea::both()
                .id_salt("regest_view_scroll")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let id = egui::Id::new("regest_view_image");
                    egui_plot::Plot::new(id)
                        .width(size.x)
                        .height(size.y)
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
                        });
                });
            let what = match self.selected_run() {
                Some(run) if self.after => format!("Fixed and run {} registered", run.id),
                _ => "Fixed and moving, before registration".into(),
            };
            let sampled = if view.step > 1 {
                format!("    display sampled 1:{}", view.step)
            } else {
                String::new()
            };
            crate::tokens::helper(ui, format!("{what}    {}{sampled}", self.overlay.label()));
        } else {
            ui.allocate_ui(egui::vec2(width, height), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Choose a moving and a fixed image");
                });
            });
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(run) = self.selected_run() else {
            return vec![("Result", "None yet".into())];
        };
        let a = run.matrix;
        let s = &run.spec.settings;
        let mut rows = vec![
            ("Run", run.id.to_string()),
            ("Technique", s.technique.label().into()),
            ("Transformation", run.class.clone()),
            ("Time", format!("{} s", super::short(run.seconds))),
            ("Correlation", super::short(run.quality)),
            (
                "Translation",
                format!("{}, {}", super::short(a[0][2]), super::short(a[1][2])),
            ),
        ];
        match s.transform {
            TransformType::Translation => {}
            TransformType::Rigid | TransformType::Similarity => {
                rows.push((
                    "Rotation",
                    format!("{}°", super::short(a[1][0].atan2(a[0][0]).to_degrees())),
                ));
                if s.transform == TransformType::Similarity {
                    rows.push(("Scale", super::short(a[0][0].hypot(a[1][0]))));
                }
            }
            TransformType::Affine => rows.push((
                "Linear part",
                format!(
                    "[{} {}; {} {}]",
                    super::short(a[0][0]),
                    super::short(a[0][1]),
                    super::short(a[1][0]),
                    super::short(a[1][1])
                ),
            )),
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let labels = [
            "Transformation variable",
            "Registered image variable",
            "Spatial reference variable",
            "Quality variable",
        ];
        for (label, name) in labels.iter().zip(self.names.iter_mut()) {
            super::name_field(ui, label, name, ws);
        }
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        self.names.to_vec()
    }
    fn summary(&self) -> Option<String> {
        self.selected_run().map(|run| {
            format!(
                "Registration Estimator: run {} ({}), correlation {}",
                run.id,
                run.spec.settings.transform.name(),
                super::short(run.quality)
            )
        })
    }
    fn export_label(&self) -> &'static str {
        "Export selected result"
    }
}

/// Source switch, variable or file, and spatial reference of one image.
/// Returns true when Browse was pressed.
fn picker_ui(
    ui: &mut egui::Ui,
    role: &str,
    title: &str,
    picker: &mut Picker,
    images: &[String],
    refs: &[String],
    controls: &mut Controls,
) -> bool {
    let mut browse = false;
    ui.label(title);
    ui.horizontal(|ui| {
        let r = crate::tokens::toggle(ui, !picker.sample && !picker.from_file, "Workspace");
        controls.note(&format!("{role}:Workspace"), &r);
        if r.clicked() {
            picker.from_file = false;
            picker.sample = false;
        }
        let r = crate::tokens::toggle(ui, !picker.sample && picker.from_file, "File");
        controls.note(&format!("{role}:File"), &r);
        if r.clicked() {
            picker.from_file = true;
            picker.sample = false;
        }
    });
    ui.add_space(crate::tokens::XS);
    if picker.sample {
        let what = if role == "moving" {
            "Sample: two blobs turned 8° and shifted [6 -4]"
        } else {
            "Sample: two Gaussian blobs, 120×120"
        };
        crate::tokens::helper(ui, what);
    } else if picker.from_file {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(&mut picker.path).char_limit(4096))
                .desired_width(ui.available_width()),
        );
        let r = ui.button("Browse…");
        controls.note(&format!("{role}:Browse"), &r);
        browse = r.clicked();
    } else {
        let r = egui::ComboBox::from_id_salt(("regest_image", role))
            .width(ui.available_width())
            .selected_text(if picker.variable.is_empty() {
                "Choose a variable"
            } else {
                &picker.variable
            })
            .show_ui(ui, |ui| {
                for name in images {
                    let r = ui.selectable_value(&mut picker.variable, name.clone(), name);
                    controls.note(&format!("{role}:{name}"), &r);
                }
            });
        controls.note(&format!("{role}:image"), &r.response);
    }
    ui.add_space(crate::tokens::XS);
    let r = egui::ComboBox::from_id_salt(("regest_ref", role))
        .width(ui.available_width())
        .selected_text(if picker.reference.is_empty() {
            "Pixel grid (no imref2d)".to_owned()
        } else {
            format!("imref2d {}", picker.reference)
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut picker.reference,
                String::new(),
                "Pixel grid (no imref2d)",
            );
            for name in refs {
                let r = ui.selectable_value(&mut picker.reference, name.clone(), name);
                controls.note(&format!("{role}:ref:{name}"), &r);
            }
        });
    controls.note(&format!("{role}:reference"), &r.response);
    browse
}

pub(crate) fn show(
    host: &mut Host<RegistrationEstimator>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    refresh(host, ui.ctx(), ws);
    let mut export = false;
    let mut generate = false;
    super::three_areas(
        ui,
        AppKind::RegistrationEstimator,
        "OVERLAY",
        |ui, area| match area {
            super::Area::Inputs => host.app.inputs_ui(ui, ws),
            super::Area::Main => host.app.main_ui(ui),
            super::Area::Output => {
                super::result_block(ui, &host.app.results());
                ui.add_space(crate::tokens::M);
                host.app.names_ui(ui, ws);
                ui.add_space(crate::tokens::S);
                let result = host.app.script(ws);
                let ready = !busy && !host.saving && result.is_ok();
                let wide = egui::vec2(ui.available_width(), crate::tokens::CONTROL);
                let r = ui.add_enabled(
                    ready,
                    egui::Button::new("Export selected result").min_size(wide),
                );
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
    if export {
        *action = host.export(ws);
    }
    if generate {
        *action = host.generate(ws, super::now());
    }
    if std::mem::take(&mut host.app.cancel_requested) {
        stop(host);
    }
    if !ui.ctx().will_discard()
        && let Some(moving) = host.app.browse_requested.take()
        && let Some(path) = pick_path(ws)
    {
        let picker = if moving {
            &mut host.app.moving
        } else {
            &mut host.app.fixed
        };
        picker.path = path;
    }
    // Process this frame's clicks (Register, Cancel, selections) now.
    refresh(host, ui.ctx(), ws);
    host.script_error = None;
    host.run_error = host
        .app
        .error
        .clone()
        .or_else(|| host.app.view_error.clone());
}

fn pick_path(ws: &Snapshot) -> Option<String> {
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
        dialog.pick_file().map(|p| p.to_string_lossy().into_owned())
    }
}

pub(crate) fn receive_open(workspace: &mut crate::Workspace) {
    let launch = workspace
        .worker
        .registration_estimator
        .lock()
        .ok()
        .and_then(|mut s| s.take());
    if let Some(launch) = launch
        && !workspace
            .worker
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        workspace.toolkits.apps.registration.app.choose(launch);
        workspace.open_app(crate::toolkits::Tool::App(AppKind::RegistrationEstimator));
    }
}
