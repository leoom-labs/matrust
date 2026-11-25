//! Classifier comparison built from public fit/predict/partition contracts.
//! One script implements validation, plots and the all-data fit. The UI reads
//! its outputs; each queued model runs in the framework's cancellable Runner.
use super::{App, AppKind, Outputs, Request, Runner, Snapshot, field, short};
use crate::toolkits::{Action, AppStatus};
use eframe::egui;
use matrust::Array;
use std::collections::VecDeque;
use std::time::Instant;

pub const MAX_MODELS: usize = 32;
pub const MAX_ROWS: usize = 10_000;
pub const MAX_PREDICTORS: usize = 64;
pub const MAX_CLASSES: usize = 32;

// Script setup names must not shadow data or user-selected outputs.
fn internal_name(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "clAccuracy",
        "clArea",
        "clBoostMethod",
        "clAuc",
        "clC",
        "clCenter",
        "clClass",
        "clClasses",
        "clCoeff",
        "clCol",
        "clComponents",
        "clConfusion",
        "clCost",
        "clDropped",
        "clEvaluated",
        "clExplained",
        "clFinal",
        "clFinalX",
        "clFirst",
        "clGrouping",
        "clFold",
        "clFoldId",
        "clFoldPred",
        "clFoldScores",
        "clFpr",
        "clGuess",
        "clKeep",
        "clLabels",
        "clLen",
        "clMean",
        "clModel",
        "clN",
        "clPartition",
        "clPrecision",
        "clPred",
        "clRecall",
        "clRocLen",
        "clRocX",
        "clRocY",
        "clRow",
        "clScale",
        "clScores",
        "clTest",
        "clTestX",
        "clTpr",
        "clTrain",
        "clTrainX",
        "clTruth",
        "clValidationRows",
        "clX",
        "clY",
    ];
    NAMES.contains(&name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    FineTree,
    MediumTree,
    CoarseTree,
    LinearDiscriminant,
    QuadraticDiscriminant,
    GaussianBayes,
    KernelBayes,
    LinearSvm,
    QuadraticSvm,
    CubicSvm,
    GaussianSvm,
    FineKnn,
    MediumKnn,
    CosineKnn,
    WeightedKnn,
    BaggedTrees,
    BoostedTrees,
    Logistic,
    Linear,
}
impl Preset {
    pub const ALL: [Self; 19] = [
        Self::FineTree,
        Self::MediumTree,
        Self::CoarseTree,
        Self::LinearDiscriminant,
        Self::QuadraticDiscriminant,
        Self::GaussianBayes,
        Self::KernelBayes,
        Self::LinearSvm,
        Self::QuadraticSvm,
        Self::CubicSvm,
        Self::GaussianSvm,
        Self::FineKnn,
        Self::MediumKnn,
        Self::CosineKnn,
        Self::WeightedKnn,
        Self::BaggedTrees,
        Self::BoostedTrees,
        Self::Logistic,
        Self::Linear,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::FineTree => "Fine tree",
            Self::MediumTree => "Medium tree",
            Self::CoarseTree => "Coarse tree",
            Self::LinearDiscriminant => "Linear discriminant",
            Self::QuadraticDiscriminant => "Quadratic discriminant",
            Self::GaussianBayes => "Gaussian naive Bayes",
            Self::KernelBayes => "Kernel naive Bayes",
            Self::LinearSvm => "Linear SVM",
            Self::QuadraticSvm => "Quadratic SVM",
            Self::CubicSvm => "Cubic SVM",
            Self::GaussianSvm => "Gaussian SVM",
            Self::FineKnn => "Fine KNN",
            Self::MediumKnn => "Medium KNN",
            Self::CosineKnn => "Cosine KNN",
            Self::WeightedKnn => "Weighted KNN",
            Self::BaggedTrees => "Bagged trees",
            Self::BoostedTrees => "Boosted trees",
            Self::Logistic => "Linear logistic",
            Self::Linear => "Linear margin",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub preset: Preset,
    pub parameter: String,
    pub leaf: String,
    pub scale: String,
    pub rate: String,
}
impl Model {
    pub fn new(preset: Preset) -> Self {
        let parameter = match preset {
            Preset::FineTree => "100",
            Preset::MediumTree => "20",
            Preset::CoarseTree => "4",
            Preset::MediumKnn | Preset::CosineKnn | Preset::WeightedKnn => "10",
            Preset::BaggedTrees | Preset::BoostedTrees => "30",
            Preset::Logistic | Preset::Linear => "0.01",
            Preset::LinearDiscriminant => "0",
            _ => "1",
        };
        Self {
            preset,
            parameter: parameter.into(),
            leaf: "1".into(),
            scale: "auto".into(),
            rate: "0.1".into(),
        }
    }
    fn parameter_label(&self) -> &'static str {
        match self.preset {
            Preset::FineTree | Preset::MediumTree | Preset::CoarseTree => "Maximum splits",
            Preset::FineKnn | Preset::MediumKnn | Preset::CosineKnn | Preset::WeightedKnn => {
                "Neighbors"
            }
            Preset::BaggedTrees | Preset::BoostedTrees => "Learning cycles",
            Preset::Logistic | Preset::Linear => "Regularization (lambda)",
            Preset::LinearDiscriminant => "Covariance shrinkage (gamma)",
            _ => "Box constraint",
        }
    }
    fn has_parameter(&self) -> bool {
        !matches!(
            self.preset,
            Preset::QuadraticDiscriminant | Preset::GaussianBayes | Preset::KernelBayes
        )
    }
    pub fn fit(&self, x: &str, y: &str) -> Result<String, String> {
        use Preset::*;
        // Typed numeric controls still go through the real single-expression parser.
        let p = numeric(&self.parameter, self.parameter_label(), 0., 20_000., false)?;
        let integer = |max| numeric(&self.parameter, self.parameter_label(), 1., max, true);
        let call = match self.preset {
            FineTree | MediumTree | CoarseTree => format!(
                "fitctree({x},{y},'MaxNumSplits',{},'MinLeafSize',{})",
                integer(20_000.)?,
                numeric(&self.leaf, "Minimum leaf size", 1., 10_000., true)?
            ),
            LinearDiscriminant => format!(
                "fitcdiscr({x},{y},'DiscrimType','linear','Gamma',{})",
                numeric(&self.parameter, "Covariance shrinkage", 0., 1., false)?
            ),
            QuadraticDiscriminant => format!("fitcdiscr({x},{y},'DiscrimType','quadratic')"),
            GaussianBayes => format!("fitcnb({x},{y},'DistributionNames','normal')"),
            KernelBayes => {
                return Err(
                    "Kernel naive Bayes: the engine supports Gaussian distributions only".into(),
                );
            }
            QuadraticSvm | CubicSvm => {
                return Err(
                    "Polynomial SVM: the engine supports linear and Gaussian kernels only".into(),
                );
            }
            LinearSvm | GaussianSvm => {
                let scale = if self.preset == GaussianSvm {
                    super::single_expression(self.scale.trim())?;
                    if self.scale.trim() == "auto" {
                        return Err("Gaussian SVM: automatic KernelScale is not supported; enter a positive scale".into());
                    }
                    format!(
                        ",'KernelScale',{}",
                        numeric(&self.scale, "Kernel scale", f64::MIN_POSITIVE, 1e10, false)?
                    )
                } else {
                    String::new()
                };
                // ECOC works for both two and many classes and preserves score order.
                format!(
                    "fitcecoc({x},{y},'Learners',templateSVM('KernelFunction','{}','BoxConstraint',{p}{scale}))",
                    if self.preset == LinearSvm {
                        "linear"
                    } else {
                        "gaussian"
                    }
                )
            }
            FineKnn | MediumKnn => format!("fitcknn({x},{y},'NumNeighbors',{})", integer(10_000.)?),
            CosineKnn => {
                return Err("Cosine KNN: this distance is not supported by the engine".into());
            }
            WeightedKnn => {
                return Err("Weighted KNN: DistanceWeight is not supported by the engine".into());
            }
            BaggedTrees | BoostedTrees => format!(
                "fitcensemble({x},{y},'Method',{},'NumLearningCycles',{},'LearnRate',{},'Learners',templateTree('MaxNumSplits',{},'MinLeafSize',{}))",
                if self.preset == BaggedTrees {
                    "'Bag'"
                } else {
                    "clBoostMethod"
                },
                integer(1000.)?,
                numeric(&self.rate, "Learning rate", f64::MIN_POSITIVE, 1., false)?,
                if self.preset == BaggedTrees { 100 } else { 1 },
                numeric(&self.leaf, "Minimum leaf size", 1., 10_000., true)?
            ),
            Logistic | Linear => format!(
                "fitclinear({x},{y},'Learner','{}','Lambda',{p})",
                if self.preset == Logistic {
                    "logistic"
                } else {
                    "svm"
                }
            ),
        };
        Ok(call)
    }
}

fn numeric(text: &str, what: &str, min: f64, max: f64, integer: bool) -> Result<String, String> {
    let text = text.trim();
    super::single_expression(text).map_err(|e| format!("{what}: {e}"))?;
    let value: f64 = text
        .parse()
        .map_err(|_| format!("{what}: enter a number"))?;
    if !value.is_finite() || value < min || value > max || (integer && value.fract() != 0.) {
        return Err(format!(
            "{what}: enter {}from {min} to {max}",
            if integer { "an integer " } else { "a number " }
        ));
    }
    Ok(super::number(value))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validation {
    KFold,
    Holdout,
    Resubstitution,
}
impl Validation {
    pub fn label(self) -> &'static str {
        match self {
            Self::KFold => "K-fold cross-validation",
            Self::Holdout => "Holdout",
            Self::Resubstitution => "Resubstitution",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub table: bool,
    pub x: String,
    pub y: String,
    pub response: String,
    pub predictors: Vec<String>,
    pub validation: Validation,
    pub folds: String,
    pub holdout: String,
    pub seed: String,
    pub standardize: bool,
    pub pca: bool,
    pub variance: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            table: false,
            x: "X".into(),
            y: "Y".into(),
            response: String::new(),
            predictors: Vec::new(),
            validation: Validation::KFold,
            folds: "5".into(),
            holdout: "25".into(),
            seed: "42".into(),
            standardize: false,
            pca: false,
            variance: "95".into(),
        }
    }
}
impl Spec {
    fn inputs(&self, ws: &Snapshot) -> Result<Vec<String>, String> {
        let expressions: Vec<&str> = if self.table {
            vec![&self.x]
        } else {
            vec![&self.x, &self.y]
        };
        let mut names = Vec::new();
        for text in expressions {
            if text.len() > 4096 || text.trim().is_empty() {
                return Err("Choose data and response expressions (up to 4096 characters)".into());
            }
            super::single_expression(text)?;
            for name in super::optimize_app::identifiers(text) {
                if ws.contains(&name) && !names.contains(&name) {
                    if internal_name(&name) {
                        return Err(
                            "This input name is reserved by the generated script; choose another variable"
                                .into(),
                        );
                    }
                    names.push(name);
                }
            }
        }
        Ok(names)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Models,
    Scatter,
    Confusion,
    Roc,
}
impl View {
    pub const ALL: [Self; 4] = [Self::Models, Self::Scatter, Self::Confusion, Self::Roc];
    pub fn label(self) -> &'static str {
        match self {
            Self::Models => "Models",
            Self::Scatter => "Scatter",
            Self::Confusion => "Confusion matrix",
            Self::Roc => "ROC",
        }
    }
}

#[derive(Clone)]
pub struct ResultView {
    pub accuracy: f64,
    pub cost: f64,
    pub dropped: usize,
    pub classes: Vec<String>,
    pub confusion: Vec<f64>,
    pub precision: Vec<f64>,
    pub recall: Vec<f64>,
    pub auc: Vec<f64>,
    pub roc: Vec<Vec<[f64; 2]>>,
    pub x: Vec<f64>,
    pub rows: usize,
    pub cols: usize,
    pub actual: Vec<f64>,
    pub predicted: Vec<f64>,
}
fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    outputs
        .get(name)
        .and_then(Array::as_numeric)
        .map(|m| m.data().to_vec())
        .ok_or_else(|| format!("Classification Learner: missing {name}"))
}
fn labels(array: &Array) -> Vec<String> {
    match array {
        Array::Numeric(m) => m.data().iter().map(|v| short(*v)).collect(),
        Array::Logical(m) => m.data().iter().map(|v| (*v != 0).to_string()).collect(),
        Array::Typed(m) => (0..m.len()).map(|i| short(m.real(i))).collect(),
        Array::Categorical(c) => (0..c.len()).map(|i| c.text(i).to_owned()).collect(),
        Array::Str(s) => s
            .data()
            .iter()
            .map(|v| v.as_ref().map_or(String::new(), |s| s.to_string()))
            .collect(),
        Array::Cell(c) => (0..c.len())
            .map(|i| {
                c.get_linear(i).map_or_else(
                    |_| "?".into(),
                    |a| match a.as_ref() {
                        Array::Character(c) => String::from_utf16_lossy(c.units()),
                        _ => "?".into(),
                    },
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}
impl ResultView {
    pub fn from_outputs(outputs: &Outputs) -> Result<Self, String> {
        let scalar = |name| {
            values(outputs, name)?
                .first()
                .copied()
                .ok_or_else(|| format!("Empty {name}"))
        };
        let classes = labels(outputs.get("clClasses").ok_or("Missing class names")?);
        let roc_x = values(outputs, "clRocX")?;
        let roc_y = values(outputs, "clRocY")?;
        let roc_len = values(outputs, "clRocLen")?;
        let capacity = outputs["clRocX"].rows();
        let roc = roc_len
            .iter()
            .enumerate()
            .map(|(k, n)| {
                (0..*n as usize)
                    .map(|i| [roc_x[i + k * capacity], roc_y[i + k * capacity]])
                    .collect()
            })
            .collect();
        let x = outputs
            .get("clX")
            .and_then(Array::as_numeric)
            .ok_or("Missing predictors")?;
        Ok(Self {
            accuracy: scalar("clAccuracy")?,
            cost: scalar("clCost")?,
            dropped: scalar("clDropped")? as usize,
            classes,
            confusion: values(outputs, "clConfusion")?,
            precision: values(outputs, "clPrecision")?,
            recall: values(outputs, "clRecall")?,
            auc: values(outputs, "clAuc")?,
            roc,
            x: x.data().to_vec(),
            rows: x.rows(),
            cols: x.cols(),
            actual: values(outputs, "clY")?,
            predicted: values(outputs, "clPred")?,
        })
    }
}
const PROBE: [&str; 16] = [
    "clAccuracy",
    "clCost",
    "clDropped",
    "clClasses",
    "clConfusion",
    "clPrecision",
    "clRecall",
    "clAuc",
    "clRocX",
    "clRocY",
    "clRocLen",
    "clX",
    "clY",
    "clPred",
    "clFoldId",
    "clScores",
];

#[derive(Clone)]
pub struct Row {
    pub model: Model,
    pub result: Option<ResultView>,
    pub error: Option<String>,
    pub seconds: Option<f64>,
    applied: Option<Spec>,
    fitted: Option<Model>,
    versions: Vec<(String, u64)>,
}
impl Row {
    fn new(preset: Preset) -> Self {
        Self {
            model: Model::new(preset),
            result: None,
            error: None,
            seconds: None,
            applied: None,
            fitted: None,
            versions: Vec::new(),
        }
    }
}

pub struct ClassificationLearner {
    pub spec: Spec,
    pub models: Vec<Row>,
    pub selected: usize,
    pub preset: Preset,
    pub view: View,
    pub sort_accuracy: bool,
    pub row_percent: bool,
    pub axes: [usize; 2],
    pub model_name: String,
    pub predictions_name: String,
    pub export_predictions: bool,
    pub message: String,
    pub(super) saving: bool,
    runner: Runner,
    queue: VecDeque<usize>,
    active: Option<(usize, Instant, Spec, Model)>,
    training_ws: Snapshot,
    training_spec: Spec,
    training_versions: Vec<(String, u64)>,
    serial: u64,
    pub stopped: bool,
}
impl Default for ClassificationLearner {
    fn default() -> Self {
        Self {
            spec: Spec::default(),
            models: Vec::new(),
            selected: 0,
            preset: Preset::MediumTree,
            view: View::Models,
            sort_accuracy: false,
            row_percent: false,
            axes: [0, 1],
            model_name: "trainedModel".into(),
            predictions_name: "validationPredictions".into(),
            export_predictions: false,
            message: String::new(),
            saving: false,
            runner: Runner::default(),
            queue: VecDeque::new(),
            active: None,
            training_ws: Snapshot::default(),
            training_spec: Spec::default(),
            training_versions: Vec::new(),
            serial: 0,
            stopped: false,
        }
    }
}

impl ClassificationLearner {
    pub fn add(&mut self, preset: Preset) {
        if self.models.len() < MAX_MODELS {
            self.models.push(Row::new(preset));
            self.selected = self.models.len() - 1;
        }
    }
    pub fn running(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty() || self.runner.running()
    }
    pub fn train(&mut self, ws: &Snapshot, all: bool) {
        self.stop();
        self.stopped = false;
        self.message.clear();
        self.training_spec = self.spec.clone();
        self.training_versions = self
            .spec
            .inputs(ws)
            .unwrap_or_default()
            .into_iter()
            .map(|n| {
                let v = ws.version(&n).unwrap_or(0);
                (n, v)
            })
            .collect();
        let vars = ws
            .variables()
            .map(|(n, a)| (n.to_owned(), a.clone()))
            .collect();
        self.training_ws.sync(&vars);
        self.training_ws.set_folder(ws.folder.clone());
        self.runner.set_folder(ws.folder());
        self.queue = if all {
            (0..self.models.len()).collect()
        } else if self.selected < self.models.len() {
            VecDeque::from([self.selected])
        } else {
            VecDeque::new()
        };
    }
    pub fn stop(&mut self) {
        if let Some((index, _, _, _)) = self.active.take() {
            self.models[index].error = Some("Stopped".into());
        }
        self.queue.clear();
        self.runner.stop();
        self.stopped = true;
    }
    /// Advance at most one model per frame; errors never discard earlier plots.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.runner.poll(ctx);
        if self.active.is_some()
            && let Some(result) = self.runner.result()
        {
            let parsed = result
                .as_ref()
                .map_err(Clone::clone)
                .and_then(ResultView::from_outputs);
            let (index, start, spec, model) = self.active.take().unwrap();
            let row = &mut self.models[index];
            row.seconds = Some(start.elapsed().as_secs_f64());
            match parsed {
                Ok(result) => {
                    self.message = format!(
                        "{}: {:.2}% accuracy · {} missing responses dropped",
                        model.preset.label(),
                        result.accuracy,
                        result.dropped
                    );
                    row.result = Some(result);
                    row.error = None;
                    row.versions = self.training_versions.clone();
                    row.applied = Some(spec);
                    row.fitted = Some(model);
                }
                Err(error) => {
                    self.message = error.clone();
                    row.error = Some(error);
                }
            }
            self.runner.stop();
        }
        if self.active.is_none()
            && let Some(index) = self.queue.pop_front()
        {
            let spec = self.training_spec.clone();
            let model = self.models[index].model.clone();
            self.serial += 1;
            let request = script_text(&spec, &model, &self.training_ws, "trainedModel", None)
                .and_then(|code| {
                    let inputs = spec
                        .inputs(&self.training_ws)?
                        .into_iter()
                        .map(|name| {
                            self.training_ws
                                .get(&name)
                                .cloned()
                                .map(|a| (name, a))
                                .ok_or("Missing input".into())
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    Ok(Request {
                        key: self.serial,
                        inputs,
                        code,
                        outputs: PROBE.iter().map(|s| (*s).into()).collect(),
                    })
                });
            match request {
                Ok(request) => {
                    self.models[index].error = None;
                    self.active = Some((index, Instant::now(), spec, model));
                    self.runner.submit(ctx, request);
                }
                Err(error) => {
                    self.message = error.clone();
                    self.models[index].error = Some(error);
                    ctx.request_repaint();
                }
            }
        }
    }
    pub fn status(&self) -> (String, AppStatus) {
        if self.active.is_some() || !self.queue.is_empty() {
            return (
                format!(
                    "Classification Learner: training · {} queued",
                    self.queue.len()
                ),
                AppStatus::Plain,
            );
        }
        if let Some(error) = self.models.iter().rev().find_map(|r| r.error.as_ref()) {
            return (format!("Classification Learner: {error}"), AppStatus::Error);
        }
        if self.stopped {
            return ("Classification Learner: stopped".into(), AppStatus::Plain);
        }
        (
            if self.message.is_empty() {
                "Classification Learner".into()
            } else {
                self.message.clone()
            },
            AppStatus::Success,
        )
    }
    fn current(&self) -> Option<&Row> {
        self.models.get(self.selected)
    }
    fn good(&self) -> Option<&ResultView> {
        self.current()?.result.as_ref()
    }
    pub fn names(&self, ws: &Snapshot, spec: &Spec) -> Result<Vec<String>, String> {
        let mut names = vec![self.model_name.trim().to_owned()];
        if self.export_predictions {
            names.push(self.predictions_name.trim().to_owned());
        }
        let inputs = spec.inputs(ws)?;
        for (i, name) in names.iter().enumerate() {
            super::single_expression(name)?;
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) || name == "end" || internal_name(name) {
                return Err(
                    "Choose output names distinct from the generated script variables".into(),
                );
            }
            if names[..i].contains(name) || inputs.contains(name) {
                return Err("Output names must differ and must not replace inputs".into());
            }
        }
        Ok(names)
    }
    pub fn export(&mut self, ws: &Snapshot) -> Option<Action> {
        let script = self.script(ws).map_err(|e| self.message = e).ok()?;
        let names = self.exported(ws);
        self.message = format!("Exported {} to the workspace", names.join(", "));
        Some(Action::ExportScript(super::all_or_nothing(&script, &names)))
    }
    pub fn generate(&self, ws: &Snapshot, time: u64) -> Option<Action> {
        self.script(ws).ok().map(|script| {
            Action::SaveAppScript(
                AppKind::ClassificationLearner,
                super::saved_script(AppKind::ClassificationLearner, time, &script),
            )
        })
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        ws: &Snapshot,
        busy: bool,
        action: &mut Option<Action>,
    ) {
        self.refresh(ui.ctx());
        let ready = !busy && !self.saving && !self.running() && self.script(ws).is_ok();
        let mut click = None;
        super::three_areas(
            ui,
            AppKind::ClassificationLearner,
            "VALIDATION",
            |ui, area| match area {
                super::Area::Inputs => self.inputs_ui(ui, ws),
                super::Area::Main => self.main_ui(ui),
                super::Area::Output => {
                    super::result_block(ui, &self.results());
                    ui.add_space(crate::tokens::S);
                    self.names_ui(ui, ws);
                    ui.add_space(crate::tokens::S);
                    click = super::output_buttons(ui, ready);
                }
            },
        );
        match click {
            Some(super::OutputClick::Export) => *action = self.export(ws),
            Some(super::OutputClick::Script) => *action = self.generate(ws, super::now()),
            None => {}
        }
    }
}

/// The sole script builder. All validation work and the final all-data fit
/// appear here, so preview, export and saved scripts execute identical code.
pub fn script_text(
    spec: &Spec,
    model: &Model,
    ws: &Snapshot,
    name: &str,
    predictions: Option<&str>,
) -> Result<String, String> {
    let inputs = spec.inputs(ws)?;
    let mut output_names = vec![name];
    if let Some(predictions) = predictions {
        output_names.push(predictions);
    }
    for (i, output) in output_names.iter().enumerate() {
        super::single_expression(output)?;
        super::check_export_name(output)?;
        if !matrust::files::valid_name(output)
            || *output == "end"
            || internal_name(output)
            || inputs.iter().any(|n| n.as_str() == *output)
            || output_names[..i].contains(output)
        {
            return Err(
                "Output names must be valid and distinct from inputs and script variables".into(),
            );
        }
    }
    let seed = numeric(&spec.seed, "Seed", 0., u32::MAX as f64, true)?;
    let variance = if spec.pca {
        numeric(
            &spec.variance,
            "Explained variance (%)",
            f64::MIN_POSITIVE,
            100.,
            false,
        )?
    } else {
        "100".into()
    };
    let fit = model.fit("clTrainX", "clY(clTrain)")?;
    let final_fit = model.fit("clFinalX", "clLabels")?;
    let (x, y) = if spec.table {
        let Some(Array::Table(table)) = ws.get(spec.x.trim()) else {
            return Err("Choose a workspace table".into());
        };
        if !table.names().contains(&spec.response) || spec.predictors.is_empty() {
            return Err("Choose a response and tick numeric predictors".into());
        }
        let mut columns = Vec::new();
        for col in &spec.predictors {
            super::single_expression(col)?;
            if !matrust::files::valid_name(col) || col == &spec.response {
                return Err(
                    "Predictors must be numeric variables distinct from the response".into(),
                );
            }
            let i = table.index_of(col).ok_or("Predictor no longer exists")?;
            if !numeric_array(table.column(i)) || table.column(i).cols() != 1 {
                return Err("Tick scalar numeric table variables".into());
            }
            columns.push(format!("{}.{}", spec.x.trim(), col));
        }
        super::single_expression(&spec.response)?;
        if !matrust::files::valid_name(&spec.response) {
            return Err("Invalid response variable".into());
        }
        (
            format!("[{}]", columns.join(" ")),
            format!("{}.{}", spec.x.trim(), spec.response),
        )
    } else {
        (
            format!("({})", spec.x.trim()),
            format!("({})", spec.y.trim()),
        )
    };
    // Known arrays get an early bound as well as the script's dynamic guard.
    if let Some(a) = ws.get(spec.x.trim())
        && (a.rows() > MAX_ROWS || (!spec.table && a.cols() > MAX_PREDICTORS))
    {
        return Err(format!(
            "Data limit: {MAX_ROWS} rows and {MAX_PREDICTORS} predictors"
        ));
    }
    let partition = match spec.validation {
        Validation::KFold => format!(
            "clPartition = cvpartition(clY,'KFold',{});",
            numeric(&spec.folds, "Folds", 2., 100., true)?
        ),
        Validation::Holdout => format!(
            "clPartition = cvpartition(clY,'Holdout',({})/100);",
            numeric(&spec.holdout, "Holdout (%)", 0.01, 99.99, false)?
        ),
        Validation::Resubstitution => "clPartition = cvpartition(clY,'Resubstitution');".into(),
    };
    let mut out = format!(
        r#"% Classification Learner: {}
% Validation fits preprocessing on training rows only.
rng({seed});
clX = {x};
clLabels = {y};
assert(isnumeric(clX) || islogical(clX),'Predictors must be numeric');
assert(ismatrix(clX),'Predictors must be a matrix');
assert(isvector(clLabels),'Response must be a label vector');
clLabels = clLabels(:);
assert(size(clX,1)==numel(clLabels),'Predictor and response row counts must match');
assert(size(clX,1)<={MAX_ROWS} && size(clX,2)<={MAX_PREDICTORS} && size(clX,2)>0,'Data exceeds app limits');
if isa(clLabels,'logical')
clKeep = true(size(clLabels));
else
clKeep = ~ismissing(clLabels);
end
clDropped = sum(~clKeep);
clX = double(clX(clKeep,:));
clLabels = clLabels(clKeep);
assert(all(isfinite(clX(:))),'Predictors must be finite');
clGrouping = clLabels;
if isa(clLabels,'string'), clGrouping = cellstr(clLabels); end
[~,clFirst,clY] = unique(clGrouping);
clClasses = clLabels(clFirst);
clY = double(clY(:));
clN = size(clX,1);
clC = numel(clClasses);
assert(clC>=2 && clC<={MAX_CLASSES},'Classification requires 2 to {MAX_CLASSES} classes');
clBoostMethod = 'AdaBoostM2';
if clC==2, clBoostMethod = 'AdaBoostM1'; end
{partition}
clPred = NaN(clN,1);
clScores = NaN(clN,clC);
clFoldId = zeros(clN,1);
for clFold = 1:clPartition.NumTestSets
clTrain = training(clPartition,clFold);
clTest = test(clPartition,clFold);
assert(numel(unique(clY(clTrain)))==clC,'Every training fold must contain every class');
clTrainX = clX(clTrain,:);
clTestX = clX(clTest,:);
"#,
        model.preset.label()
    );
    out += &preprocess(spec, "clTrainX", Some("clTestX"), &variance);
    out += &format!(
        r#"clModel = {fit};
[clFoldPred,clFoldScores] = predict(clModel,clTestX);
clPred(clTest) = clFoldPred;
clScores(clTest,:) = clFoldScores;
clFoldId(clTest) = clFold;
end
clEvaluated = ~isnan(clPred);
clTruth = clY(clEvaluated);
clGuess = clPred(clEvaluated);
clCost = sum(clTruth~=clGuess);
clAccuracy = 100*(1-clCost/numel(clTruth));
clConfusion = confusionmat(clTruth,clGuess,'Order',(1:clC)');
clPrecision = diag(clConfusion)./sum(clConfusion,1)';
clRecall = diag(clConfusion)./sum(clConfusion,2);
clAuc = NaN(clC,1);
clRocX = NaN(clN+1,clC);
clRocY = NaN(clN+1,clC);
clRocLen = zeros(clC,1);
for clClass = 1:clC
if any(clTruth==clClass) && any(clTruth~=clClass)
[clFpr,clTpr,~,clArea] = perfcurve(clTruth,clScores(clEvaluated,clClass),clClass);
clLen = numel(clFpr);
clRocX(1:clLen,clClass) = clFpr;
clRocY(1:clLen,clClass) = clTpr;
clRocLen(clClass) = clLen;
clAuc(clClass) = clArea;
end
end
% Confusion counts: rows are actual classes, columns are predictions.
figure; imagesc(clConfusion); xlabel('Predicted class'); ylabel('Actual class'); title('Validation counts');
for clRow=1:clC
for clCol=1:clC
text(clCol,clRow,num2str(clConfusion(clRow,clCol)));
end
end
figure; hold on;
for clClass=1:clC
clLen=clRocLen(clClass);
if clLen>0
plot(clRocX(1:clLen,clClass),clRocY(1:clLen,clClass));
end
end
xlabel('False positive rate'); ylabel('True positive rate'); title('One versus rest ROC'); hold off;
% Final model uses every retained response, independently of validation.
clFinalX = clX;
"#
    );
    out += &preprocess(spec, "clFinalX", None, &variance);
    out += &format!("clFinal = {final_fit};\n");
    if spec.standardize || spec.pca {
        let mut transform = "X".to_owned();
        if spec.standardize {
            transform = format!("bsxfun(@rdivide,bsxfun(@minus,{transform},clMean),clScale)");
        }
        if spec.pca {
            transform = format!("(bsxfun(@minus,{transform},clCenter)*clCoeff)");
        }
        out += &format!(
            "{name} = struct('ClassificationModel',clFinal,'predictFcn',@(X) predict(clFinal,{transform}));\n"
        );
    } else {
        out += &format!("{name} = clFinal;\n");
    }
    if let Some(predictions) = predictions {
        // Only evaluated rows are returned; clValidationRows maps to cleaned input.
        out += &format!(
            "{predictions} = clClasses(clGuess);\nclValidationRows = find(clEvaluated);\n"
        );
    }
    Ok(out)
}
fn preprocess(spec: &Spec, train: &str, test: Option<&str>, variance: &str) -> String {
    let mut out = String::new();
    if spec.standardize {
        out += &format!(
            "clMean = mean({train},1);\nclScale = std({train},0,1);\nclScale(clScale==0) = 1;\n{train} = bsxfun(@rdivide,bsxfun(@minus,{train},clMean),clScale);\n"
        );
        if let Some(test) = test {
            out += &format!("{test} = bsxfun(@rdivide,bsxfun(@minus,{test},clMean),clScale);\n");
        }
    }
    if spec.pca {
        out += &format!(
            "[clCoeff,~,~,~,clExplained,clCenter] = pca({train});\nclComponents = find(cumsum(clExplained)>={variance},1);\nif isempty(clComponents), clComponents = size(clCoeff,2); end\nclCoeff = clCoeff(:,1:clComponents);\n{train} = bsxfun(@minus,{train},clCenter)*clCoeff;\n"
        );
        if let Some(test) = test {
            out += &format!("{test} = bsxfun(@minus,{test},clCenter)*clCoeff;\n");
        }
    }
    out
}
fn numeric_array(a: &Array) -> bool {
    matches!(a, Array::Numeric(_) | Array::Typed(_) | Array::Logical(_)) && !a.is_nd()
}

impl App for ClassificationLearner {
    fn kind(&self) -> AppKind {
        AppKind::ClassificationLearner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let row = self.current().ok_or("Add a model, then train it")?;
        let spec = row
            .applied
            .as_ref()
            .ok_or("Train the selected model first")?;
        let model = row
            .fitted
            .as_ref()
            .ok_or("Train the selected model first")?;
        if row.error.is_some() {
            return Err("The selected model failed; fix its inputs and train again".into());
        }
        if &self.spec != spec
            || &row.model != model
            || row.versions.iter().any(|(n, v)| ws.version(n) != Some(*v))
        {
            return Err("Inputs changed; train again before exporting".into());
        }
        let names = self.names(ws, spec)?;
        script_text(spec, model, ws, &names[0], names.get(1).map(String::as_str))
    }
    fn inputs(&self) -> Vec<String> {
        self.current()
            .map(|r| r.versions.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default()
    }
    fn probe(&self, _ws: &Snapshot) -> (String, Vec<String>) {
        (String::new(), PROBE.iter().map(|s| (*s).into()).collect())
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let result = ResultView::from_outputs(outputs)?;
        if let Some(row) = self.models.get_mut(self.selected) {
            row.result = Some(result);
        }
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.running() && !self.models.is_empty(),
                    egui::Button::new("Train selected"),
                )
                .clicked()
            {
                self.train(ws, false);
            }
            if ui
                .add_enabled(
                    !self.running() && !self.models.is_empty(),
                    egui::Button::new("Train all"),
                )
                .clicked()
            {
                self.train(ws, true);
            }
            if ui
                .add_enabled(self.running(), egui::Button::new("Stop"))
                .clicked()
            {
                self.stop();
            }
        });
        ui.add_space(crate::tokens::S);
        // Freeze editable controls while a batch is queued; Stop is always live.
        let editable = !self.running();
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.spec.table, false, "Matrix");
                ui.selectable_value(&mut self.spec.table, true, "Table");
            });
            if self.spec.table {
                let old = self.spec.x.clone();
                field(ui, "Workspace table", |ui| {
                    egui::ComboBox::from_id_salt("cl_table")
                        .selected_text(&self.spec.x)
                        .show_ui(ui, |ui| {
                            for (n, a) in ws.variables() {
                                if matches!(a, Array::Table(_)) {
                                    ui.selectable_value(&mut self.spec.x, n.to_owned(), n);
                                }
                            }
                        });
                });
                if old != self.spec.x {
                    self.spec.predictors.clear();
                    self.spec.response.clear();
                }
                if let Some(Array::Table(table)) = ws.get(&self.spec.x) {
                    field(ui, "Response", |ui| {
                        egui::ComboBox::from_id_salt("cl_response")
                            .selected_text(&self.spec.response)
                            .show_ui(ui, |ui| {
                                for n in table.names() {
                                    ui.selectable_value(&mut self.spec.response, n.clone(), n);
                                }
                            });
                    });
                    ui.label("Numeric predictors");
                    for (n, a) in table.names().iter().zip(table.columns()) {
                        if n == &self.spec.response {
                            self.spec.predictors.retain(|p| p != n);
                            continue;
                        }
                        if numeric_array(a) && a.cols() == 1 {
                            let mut chosen = self.spec.predictors.contains(n);
                            if ui.checkbox(&mut chosen, n).changed() {
                                if chosen {
                                    self.spec.predictors.push(n.clone());
                                } else {
                                    self.spec.predictors.retain(|p| p != n);
                                }
                            }
                        }
                    }
                }
            } else {
                expression_field(ui, "Predictor matrix X", &mut self.spec.x);
                expression_field(ui, "Response vector Y", &mut self.spec.y);
            }
            field(ui, "Validation", |ui| {
                egui::ComboBox::from_id_salt("cl_validation")
                    .selected_text(self.spec.validation.label())
                    .show_ui(ui, |ui| {
                        for v in [
                            Validation::KFold,
                            Validation::Holdout,
                            Validation::Resubstitution,
                        ] {
                            ui.selectable_value(&mut self.spec.validation, v, v.label());
                        }
                    });
            });
            match self.spec.validation {
                Validation::KFold => expression_field(ui, "Folds", &mut self.spec.folds),
                Validation::Holdout => expression_field(ui, "Holdout (%)", &mut self.spec.holdout),
                Validation::Resubstitution => {}
            }
            expression_field(ui, "Random seed", &mut self.spec.seed);
            ui.checkbox(&mut self.spec.standardize, "Standardize predictors");
            ui.checkbox(&mut self.spec.pca, "PCA within each training fold");
            if self.spec.pca {
                expression_field(ui, "Explained variance (%)", &mut self.spec.variance);
            }
            ui.add_space(crate::tokens::S);
            field(ui, "Model preset", |ui| {
                egui::ComboBox::from_id_salt("cl_preset")
                    .selected_text(self.preset.label())
                    .show_ui(ui, |ui| {
                        for p in Preset::ALL {
                            ui.selectable_value(&mut self.preset, p, p.label());
                        }
                    });
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.models.len() < MAX_MODELS,
                        egui::Button::new("Add model"),
                    )
                    .clicked()
                {
                    self.add(self.preset);
                }
                if ui
                    .add_enabled(!self.models.is_empty(), egui::Button::new("Remove"))
                    .clicked()
                {
                    self.models.remove(self.selected.min(self.models.len() - 1));
                    self.selected = self.selected.min(self.models.len().saturating_sub(1));
                }
            });
            if let Some(row) = self.models.get_mut(self.selected) {
                ui.label(row.model.preset.label());
                if row.model.has_parameter() {
                    let label = row.model.parameter_label();
                    expression_field(ui, label, &mut row.model.parameter);
                }
                if matches!(
                    row.model.preset,
                    Preset::FineTree
                        | Preset::MediumTree
                        | Preset::CoarseTree
                        | Preset::BaggedTrees
                        | Preset::BoostedTrees
                ) {
                    expression_field(ui, "Minimum leaf size", &mut row.model.leaf);
                }
                if row.model.preset == Preset::GaussianSvm {
                    expression_field(ui, "Kernel scale (auto unavailable)", &mut row.model.scale);
                }
                if row.model.preset == Preset::BoostedTrees {
                    expression_field(ui, "Learning rate", &mut row.model.rate);
                }
                if let Err(e) = row.model.fit("X", "Y") {
                    super::expression_message(ui, Err(e));
                }
            }
        });
    }
    fn main_caption(&self) -> &'static str {
        "VALIDATION"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for v in View::ALL {
                if crate::tokens::tab(ui, self.view == v, v.label()).clicked() {
                    self.view = v;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        if self.view == View::Models {
            ui.checkbox(&mut self.sort_accuracy, "Sort by accuracy");
            let mut order: Vec<usize> = (0..self.models.len()).collect();
            if self.sort_accuracy {
                order.sort_by(|a, b| {
                    self.models[*b]
                        .result
                        .as_ref()
                        .map_or(f64::NEG_INFINITY, |r| r.accuracy)
                        .total_cmp(
                            &self.models[*a]
                                .result
                                .as_ref()
                                .map_or(f64::NEG_INFINITY, |r| r.accuracy),
                        )
                });
            }
            let best = self
                .models
                .iter()
                .filter(|r| r.error.is_none())
                .filter_map(|r| r.result.as_ref().map(|r| r.accuracy))
                .max_by(f64::total_cmp);
            egui::Grid::new("cl_models")
                .striped(true)
                .spacing(egui::vec2(crate::tokens::S, crate::tokens::S))
                .show(ui, |ui| {
                    for title in ["Model", "Accuracy (%)", "Time (s)", "Status"] {
                        ui.label(title);
                    }
                    ui.end_row();
                    for i in order {
                        let row = &self.models[i];
                        let name = format!(
                            "{}{}",
                            row.model.preset.label(),
                            if row.error.is_none()
                                && row
                                    .result
                                    .as_ref()
                                    .is_some_and(|r| Some(r.accuracy) == best)
                            {
                                " · best"
                            } else {
                                ""
                            }
                        );
                        if ui.selectable_label(self.selected == i, name).clicked() {
                            self.selected = i;
                        }
                        ui.label(
                            row.result
                                .as_ref()
                                .map_or("—".into(), |r| format!("{:.2}", r.accuracy)),
                        );
                        ui.label(row.seconds.map_or("—".into(), short));
                        let active = self.active.as_ref().is_some_and(|a| a.0 == i);
                        ui.label(if active {
                            "Training"
                        } else if self.queue.contains(&i) {
                            "Queued"
                        } else if let Some(e) = &row.error {
                            e
                        } else if row.result.is_some() {
                            "Trained"
                        } else {
                            "Ready"
                        });
                        ui.end_row();
                    }
                });
            if self.models.is_empty() {
                crate::tokens::helper(ui, "Add a preset in Inputs, then train selected models.");
            }
            return;
        }
        if self.good().is_none() {
            crate::tokens::helper(ui, "Train a model to view its validation results.");
            return;
        }
        if self.current().is_some_and(|r| r.error.is_some()) {
            crate::tokens::helper(ui, "Showing the last successful result.");
        }
        let height = ui.available_height().max(120.);
        let p = crate::tokens::palette(ui.ctx());
        match self.view {
            View::Scatter => {
                let cols = self.good().unwrap().cols;
                for (axis, label) in ["Horizontal predictor", "Vertical predictor"]
                    .iter()
                    .enumerate()
                {
                    self.axes[axis] = self.axes[axis].min(cols.saturating_sub(1));
                    egui::ComboBox::from_id_salt(("cl_axis", axis))
                        .selected_text(format!("{} {}", label, self.axes[axis] + 1))
                        .show_ui(ui, |ui| {
                            for j in 0..cols {
                                ui.selectable_value(
                                    &mut self.axes[axis],
                                    j,
                                    format!("Predictor {}", j + 1),
                                );
                            }
                        });
                }
                let r = self.good().unwrap();
                chart(
                    "cl_scatter",
                    (height - 2. * crate::tokens::CONTROL).max(80.),
                    "Predictor",
                    "Predictor",
                )
                .legend(egui_plot::Legend::default())
                .show(ui, |plot| {
                    for k in 0..r.classes.len() {
                        let pts = (0..r.rows)
                            .filter(|i| r.actual[*i] == (k + 1) as f64)
                            .take(5000)
                            .map(|i| {
                                [
                                    r.x[i + self.axes[0] * r.rows],
                                    r.x[i + self.axes[1] * r.rows],
                                ]
                            })
                            .collect::<Vec<_>>();
                        plot.points(
                            egui_plot::Points::new(&r.classes[k], pts)
                                .radius(3.)
                                .shape(marker(k))
                                .color(
                                    p.accent_fill
                                        .lerp_to_gamma(p.surface_sunken, (k % 5) as f32 * 0.13),
                                )
                                .filled(k % 2 == 0),
                        );
                    }
                    let bad = (0..r.rows)
                        .filter(|i| r.predicted[*i].is_finite() && r.actual[*i] != r.predicted[*i])
                        .map(|i| {
                            [
                                r.x[i + self.axes[0] * r.rows],
                                r.x[i + self.axes[1] * r.rows],
                            ]
                        })
                        .collect::<Vec<_>>();
                    plot.points(
                        egui_plot::Points::new("Misclassified", bad)
                            .shape(egui_plot::MarkerShape::Cross)
                            .color(p.text)
                            .radius(6.),
                    );
                });
            }
            View::Confusion => {
                ui.checkbox(&mut self.row_percent, "Row-normalized (%)");
                let r = self.good().unwrap();
                let n = r.classes.len();
                egui::Grid::new("cl_confusion")
                    .spacing(egui::vec2(crate::tokens::S, crate::tokens::S))
                    .show(ui, |ui| {
                        ui.label("Actual / predicted");
                        for name in &r.classes {
                            ui.label(name);
                        }
                        ui.end_row();
                        for i in 0..n {
                            ui.label(&r.classes[i]);
                            let sum = (0..n).map(|j| r.confusion[i + j * n]).sum::<f64>();
                            for j in 0..n {
                                let count = r.confusion[i + j * n];
                                let value = if self.row_percent {
                                    if sum > 0. {
                                        100. * count / sum
                                    } else {
                                        f64::NAN
                                    }
                                } else {
                                    count
                                };
                                let tint = if sum > 0. { (count / sum) as f32 } else { 0. };
                                egui::Frame::new()
                                    .fill(p.surface_sunken.lerp_to_gamma(p.accent_fill, tint * 0.6))
                                    .inner_margin(crate::tokens::S)
                                    .show(ui, |ui| {
                                        ui.label(short(value));
                                    });
                            }
                            ui.end_row();
                        }
                    });
            }
            View::Roc => {
                let r = self.good().unwrap();
                chart(
                    "cl_roc",
                    height,
                    "False positive rate",
                    "True positive rate",
                )
                .legend(egui_plot::Legend::default())
                .show(ui, |plot| {
                    for (k, curve) in r.roc.iter().enumerate() {
                        plot.line(
                            egui_plot::Line::new(
                                format!("{} · AUC {}", r.classes[k], short(r.auc[k])),
                                curve.clone(),
                            )
                            .color(p.accent_fill)
                            .style(if k % 2 == 0 {
                                egui_plot::LineStyle::Solid
                            } else {
                                egui_plot::LineStyle::dashed_dense()
                            }),
                        );
                    }
                });
            }
            View::Models => {}
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(r) = self.good() else {
            return vec![("Selected model", "No trained model".into())];
        };
        let row = self.current().unwrap();
        vec![
            (
                "Selected model",
                row.fitted
                    .as_ref()
                    .unwrap_or(&row.model)
                    .preset
                    .label()
                    .into(),
            ),
            ("Accuracy", format!("{:.2}%", r.accuracy)),
            ("Misclassification cost", short(r.cost)),
            (
                "Validation",
                row.applied.as_ref().map_or_else(
                    || "—".into(),
                    |s| match s.validation {
                        Validation::KFold => format!("{} folds", s.folds),
                        Validation::Holdout => format!("{}% holdout", s.holdout),
                        Validation::Resubstitution => "Resubstitution".into(),
                    },
                ),
            ),
            ("Missing responses dropped", r.dropped.to_string()),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if let Some(r) = self.good() {
            ui.label("Class · precision · recall");
            for (k, name) in r.classes.iter().enumerate() {
                ui.label(format!(
                    "{name} · {} · {}",
                    short(r.precision[k]),
                    short(r.recall[k])
                ));
            }
        }
        if let Some(row) = self.current() {
            let model = row.fitted.as_ref().unwrap_or(&row.model);
            crate::tokens::helper(ui, model.fit("X", "Y").unwrap_or_else(|e| e));
            if let Some(spec) = &row.applied {
                crate::tokens::helper(
                    ui,
                    format!(
                        "Seed {} · standardize {} · PCA {}",
                        spec.seed,
                        spec.standardize,
                        if spec.pca {
                            format!("{}%", spec.variance)
                        } else {
                            "off".into()
                        }
                    ),
                );
            }
        }
        ui.add_space(crate::tokens::S);
        super::name_field(ui, "Model name", &mut self.model_name, ws);
        ui.checkbox(
            &mut self.export_predictions,
            "Export validation predictions",
        );
        if self.export_predictions {
            super::name_field(ui, "Predictions name", &mut self.predictions_name, ws);
        }
        if let Err(e) = self.script(ws) {
            super::expression_message(ui, Err(e));
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.current()
            .and_then(|r| r.applied.as_ref())
            .and_then(|s| self.names(ws, s).ok())
            .unwrap_or_default()
    }
    fn summary(&self) -> Option<String> {
        self.good().map(|r| {
            format!(
                "{:.2}% accuracy · {} missing responses dropped",
                r.accuracy, r.dropped
            )
        })
    }
}
fn expression_field(ui: &mut egui::Ui, label: &str, text: &mut String) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(text).char_limit(4096))
                .desired_width(ui.available_width()),
        );
        super::expression_message(ui, super::single_expression(text));
    });
}
fn chart<'a>(id: &str, height: f32, x: &str, y: &str) -> egui_plot::Plot<'a> {
    egui_plot::Plot::new(id.to_owned())
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .show_background(false)
        .x_axis_label(x)
        .y_axis_label(y)
}
fn marker(i: usize) -> egui_plot::MarkerShape {
    use egui_plot::MarkerShape;
    [
        MarkerShape::Circle,
        MarkerShape::Square,
        MarkerShape::Diamond,
        MarkerShape::Plus,
    ][i % 4]
}
