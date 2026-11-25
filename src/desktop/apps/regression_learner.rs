//! Regression model comparison. Every numerical result is computed by the same
//! MATLAB script used for preview, workspace export and saved scripts.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, short};
use crate::theme::ThemedScrollArea;
use crate::toolkits::{Action, AppStatus};
use eframe::egui;
use matrust::Array;
use std::collections::{BTreeMap, VecDeque};

pub const MAX_ROWS: usize = 20_000;
pub const MAX_PREDICTORS: usize = 128;
pub const MAX_MODELS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Linear,
    Interactions,
    Robust,
    Stepwise,
    FineTree,
    MediumTree,
    CoarseTree,
    LinearSvm,
    QuadraticSvm,
    CubicSvm,
    GaussianSvm,
    SquaredExponentialGp,
    MaternGp,
    ExponentialGp,
    RationalQuadraticGp,
    Boosted,
    Bagged,
    LinearLeastSquares,
    LinearHighSvm,
}
impl Preset {
    pub const ALL: [Self; 19] = [
        Self::Linear,
        Self::Interactions,
        Self::Robust,
        Self::Stepwise,
        Self::FineTree,
        Self::MediumTree,
        Self::CoarseTree,
        Self::LinearSvm,
        Self::QuadraticSvm,
        Self::CubicSvm,
        Self::GaussianSvm,
        Self::SquaredExponentialGp,
        Self::MaternGp,
        Self::ExponentialGp,
        Self::RationalQuadraticGp,
        Self::Boosted,
        Self::Bagged,
        Self::LinearLeastSquares,
        Self::LinearHighSvm,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Interactions => "Linear interactions",
            Self::Robust => "Robust linear",
            Self::Stepwise => "Stepwise linear",
            Self::FineTree => "Fine tree",
            Self::MediumTree => "Medium tree",
            Self::CoarseTree => "Coarse tree",
            Self::LinearSvm => "Linear SVM",
            Self::QuadraticSvm => "Quadratic SVM",
            Self::CubicSvm => "Cubic SVM",
            Self::GaussianSvm => "Gaussian SVM",
            Self::SquaredExponentialGp => "Squared exponential GP",
            Self::MaternGp => "Matern 5/2 GP",
            Self::ExponentialGp => "Exponential GP",
            Self::RationalQuadraticGp => "Rational quadratic GP",
            Self::Boosted => "Boosted trees",
            Self::Bagged => "Bagged trees",
            Self::LinearLeastSquares => "High-dimensional least squares",
            Self::LinearHighSvm => "High-dimensional SVM",
        }
    }
    fn tree(self) -> bool {
        matches!(
            self,
            Self::FineTree | Self::MediumTree | Self::CoarseTree | Self::Boosted | Self::Bagged
        )
    }
    fn svm(self) -> bool {
        matches!(
            self,
            Self::LinearSvm | Self::QuadraticSvm | Self::CubicSvm | Self::GaussianSvm
        )
    }
    fn gp(self) -> bool {
        matches!(
            self,
            Self::SquaredExponentialGp
                | Self::MaternGp
                | Self::ExponentialGp
                | Self::RationalQuadraticGp
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelSpec {
    pub preset: Preset,
    pub leaf: String,
    pub robust_tune: String,
    pub cycles: String,
    pub rate: String,
    pub box_constraint: String,
    pub epsilon: String,
    pub scale: String,
    pub sigma: String,
    pub length_scale: String,
    pub signal_scale: String,
    pub shape: String,
    pub gp_linear_basis: bool,
    pub lambda: String,
}
impl ModelSpec {
    pub fn new(preset: Preset) -> Self {
        Self {
            preset,
            leaf: match preset {
                Preset::FineTree => "4",
                Preset::CoarseTree => "36",
                _ => "12",
            }
            .into(),
            robust_tune: "4.685".into(),
            cycles: "30".into(),
            rate: "0.1".into(),
            box_constraint: "1".into(),
            epsilon: "0.1".into(),
            scale: "1".into(),
            sigma: "0.1".into(),
            length_scale: "10".into(),
            signal_scale: "100".into(),
            shape: "1".into(),
            gp_linear_basis: false,
            lambda: "0.01".into(),
        }
    }
    pub fn fit_call(&self, x: &str, y: &str) -> Result<String, String> {
        use Preset::*;
        let call = match self.preset {
            Linear => format!("fitlm({x}, {y}, 'linear')"),
            Interactions => format!("fitlm({x}, {y}, 'interactions')"),
            Robust => {
                let tune = parameter(
                    &self.robust_tune,
                    "Robust tuning constant",
                    0.1,
                    100.,
                    false,
                )?;
                format!("regressionLearnerRobust({x}, {y}, {tune})")
            }
            Stepwise => format!("stepwiselm({x}, {y}, 'linear', 'Upper', 'interactions')"),
            p if p.tree() => {
                let leaf = parameter(&self.leaf, "Minimum leaf size", 1., 20_000., true)?;
                if matches!(p, Boosted | Bagged) {
                    let cycles = parameter(&self.cycles, "Learning cycles", 1., 1000., true)?;
                    let rate =
                        parameter(&self.rate, "Learning rate", f64::MIN_POSITIVE, 1., false)?;
                    format!(
                        "fitrensemble({x}, {y}, 'Method', '{}', 'Learners', templateTree('MinLeafSize', {leaf}), 'NumLearningCycles', {cycles}, 'LearnRate', {rate})",
                        if p == Bagged { "Bag" } else { "LSBoost" }
                    )
                } else {
                    format!("fitrtree({x}, {y}, 'MinLeafSize', {leaf})")
                }
            }
            p if p.svm() => {
                let c = parameter(
                    &self.box_constraint,
                    "Box constraint",
                    f64::MIN_POSITIVE,
                    1e9,
                    false,
                )?;
                let epsilon = parameter(&self.epsilon, "Epsilon", 0., 1e9, false)?;
                let scale = parameter(&self.scale, "Kernel scale", f64::MIN_POSITIVE, 1e9, false)?;
                let kernel = match p {
                    LinearSvm => "linear",
                    GaussianSvm => "gaussian",
                    _ => "polynomial",
                };
                let order = match p {
                    QuadraticSvm => ", 'PolynomialOrder', 2",
                    CubicSvm => ", 'PolynomialOrder', 3",
                    _ => "",
                };
                format!(
                    "fitrsvm({x}, {y}, 'KernelFunction', '{kernel}'{order}, 'BoxConstraint', {c}, 'Epsilon', {epsilon}, 'KernelScale', {scale})"
                )
            }
            p if p.gp() => {
                let basis = if self.gp_linear_basis {
                    "linear"
                } else {
                    "constant"
                };
                let sigma = parameter(&self.sigma, "Noise sigma", f64::MIN_POSITIVE, 1e9, false)?;
                let length = parameter(
                    &self.length_scale,
                    "Length scale",
                    f64::MIN_POSITIVE,
                    1e9,
                    false,
                )?;
                let signal = parameter(
                    &self.signal_scale,
                    "Signal scale",
                    f64::MIN_POSITIVE,
                    1e9,
                    false,
                )?;
                let parameters = if p == RationalQuadraticGp {
                    let shape = parameter(&self.shape, "Shape", f64::MIN_POSITIVE, 1e9, false)?;
                    format!("[{length} {shape} {signal}]")
                } else {
                    format!("[{length} {signal}]")
                };
                let kernel = match p {
                    SquaredExponentialGp => "squaredexponential",
                    MaternGp => "matern52",
                    ExponentialGp => "exponential",
                    _ => "rationalquadratic",
                };
                format!(
                    "fitrgp({x}, {y}, 'KernelFunction', '{kernel}', 'Sigma', {sigma}, 'KernelParameters', {parameters}, 'BasisFunction', '{basis}')"
                )
            }
            LinearLeastSquares | LinearHighSvm => {
                let lambda = parameter(&self.lambda, "Lambda", 0., 1e9, false)?;
                format!(
                    "fitrlinear({x}, {y}, 'Learner', '{}', 'Lambda', {lambda})",
                    if self.preset == LinearLeastSquares {
                        "leastsquares"
                    } else {
                        "svm"
                    }
                )
            }
            _ => unreachable!(),
        };
        Ok(call)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validation {
    KFold,
    Holdout,
    Resubstitution,
}
impl Validation {
    pub const ALL: [Self; 3] = [Self::KFold, Self::Holdout, Self::Resubstitution];
    pub fn label(self) -> &'static str {
        match self {
            Self::KFold => "K-fold",
            Self::Holdout => "Holdout",
            Self::Resubstitution => "Resubstitution",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub use_table: bool,
    pub table: String,
    pub response: String,
    pub predictors: Vec<String>,
    pub x: String,
    pub y: String,
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
            use_table: false,
            table: String::new(),
            response: String::new(),
            predictors: Vec::new(),
            x: "X".into(),
            y: "y".into(),
            validation: Validation::KFold,
            folds: "5".into(),
            holdout: "25".into(),
            seed: "1".into(),
            standardize: false,
            pca: false,
            variance: "95".into(),
        }
    }
}
fn expression(text: &str) -> Result<&str, String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 4000 {
        return Err("Regression Learner: enter one expression (up to 4000 characters)".into());
    }
    super::single_expression(text).map_err(|e| format!("Regression Learner: {e}"))?;
    Ok(text)
}
fn parameter(text: &str, label: &str, min: f64, max: f64, integer: bool) -> Result<String, String> {
    let text = expression(text)?;
    let n: f64 = text
        .parse()
        .map_err(|_| format!("Regression Learner: {label} must be a number"))?;
    if !n.is_finite() || n < min || n > max || (integer && n.fract() != 0.) {
        return Err(format!(
            "Regression Learner: {label} must be {}between {min} and {max}",
            if integer { "an integer " } else { "" }
        ));
    }
    Ok(super::number(n))
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
impl Spec {
    fn data(&self, ws: &Snapshot) -> Result<(String, String), String> {
        if !self.use_table {
            return Ok((expression(&self.x)?.into(), expression(&self.y)?.into()));
        }
        expression(&self.table)?;
        let columns = super::curve_fitter::table_columns(ws, &self.table);
        if !columns.iter().any(|(n, _)| n == &self.response)
            || self.predictors.is_empty()
            || self
                .predictors
                .iter()
                .any(|n| n == &self.response || !columns.iter().any(|(c, _)| c == n))
        {
            return Err(
                "Regression Learner: choose a numeric response and numeric predictors".into(),
            );
        }
        let column = |s: &str| format!("{}.({})", self.table, quote(s));
        let x = format!(
            "[{}]",
            self.predictors
                .iter()
                .map(|n| column(n))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let y = column(&self.response);
        expression(&x)?;
        expression(&y)?;
        Ok((x, y))
    }
    fn inputs(&self) -> Vec<String> {
        if self.use_table {
            return vec![self.table.clone()];
        }
        let mut names = super::optimize_app::identifiers(&format!("({}), ({})", self.x, self.y));
        names.sort();
        names.dedup();
        names
    }
    fn description(&self) -> String {
        match self.validation {
            Validation::KFold => format!("{}-fold cross-validation", self.folds),
            Validation::Holdout => format!("{}% holdout", self.holdout),
            Validation::Resubstitution => "Resubstitution".into(),
        }
    }
}

/// The only numerical program builder. The local function isolates temporary
/// names from workspace data; preprocessing is learnt separately in each fold.
pub fn script_text(
    spec: &Spec,
    model: &ModelSpec,
    ws: &Snapshot,
    model_name: &str,
    prediction_name: Option<&str>,
) -> Result<String, String> {
    let (x, y) = spec.data(ws)?;
    let inputs = spec.inputs();
    for name in std::iter::once(model_name).chain(prediction_name) {
        expression(name)?;
        if !matrust::files::valid_name(name)
            || name.starts_with("rl_")
            || name == "regressionLearnerRun"
            || name == "regressionLearnerPrepare"
            || name == "regressionLearnerRobust"
            || inputs.iter().any(|n| n == name)
        {
            return Err("Regression Learner: output names must be valid and distinct from input and helper names".into());
        }
        super::check_export_name(name)?;
    }
    if prediction_name == Some(model_name) {
        return Err("Regression Learner: output names must differ".into());
    }
    let seed = parameter(&spec.seed, "Seed", 0., u32::MAX as f64, true)?;
    let partition = match spec.validation {
        Validation::KFold => format!(
            "cvpartition(numel(y), 'KFold', {})",
            parameter(&spec.folds, "Folds", 2., 100., true)?
        ),
        Validation::Holdout => format!(
            "cvpartition(numel(y), 'Holdout', {} / 100)",
            parameter(&spec.holdout, "Holdout percent", 1., 99., false)?
        ),
        Validation::Resubstitution => "cvpartition(numel(y), 'Resubstitution')".into(),
    };
    let variance = if spec.pca {
        parameter(
            &spec.variance,
            "Explained variance percent",
            f64::MIN_POSITIVE,
            100.,
            false,
        )?
    } else {
        "100".into()
    };
    let fit = model.fit_call("trainX", "y(trainRows)")?;
    let final_fit = model.fit_call("allX", "y")?;
    let result_name = super::unused_name("rl_result", |n| {
        ws.contains(n) || n == model_name || prediction_name == Some(n)
    });
    let predictions = super::unused_name("rl_predictions", |n| {
        ws.contains(n) || n == model_name || prediction_name == Some(n)
    });
    let mut code = format!(
        "% {} — {}\n% Preprocessing uses training rows only. Holdout predictions are NaN outside the test set.\n[{model_name}, {predictions}, {result_name}] = regressionLearnerRun(({x}), ({y}));\n",
        model.preset.label(),
        spec.description()
    );
    if let Some(name) = prediction_name {
        code += &format!("{name} = {predictions};\n");
    }
    code += &format!(
        "figure; subplot(1,2,1); plot({result_name}.actual, {result_name}.predicted, 'o');\nhold on; plot({result_name}.actual, {result_name}.actual); hold off; xlabel('Actual'); ylabel('Predicted');\nsubplot(1,2,2); plot({result_name}.predicted, {result_name}.residual, 'o'); xlabel('Predicted'); ylabel('Residual');\n"
    );
    code += &format!(
        "function [trained, predictions, result] = regressionLearnerRun(X, y)\n\
        if ~isnumeric(X) || ~isreal(X) || ~ismatrix(X) || ~isnumeric(y) || ~isreal(y) || ~isvector(y), error('Regression Learner: use a real predictor matrix and response vector'); end\n\
        X = double(X); y = double(y(:));\n\
        if size(X,1) ~= numel(y), error('Regression Learner: X has %d rows, y has %d', size(X,1), numel(y)); end\n\
        if size(X,1) > {MAX_ROWS}, error('Regression Learner: X has %d rows (limit {MAX_ROWS})', size(X,1)); end\n\
        if size(X,2) < 1 || size(X,2) > {MAX_PREDICTORS}, error('Regression Learner: X has %d predictors (expected 1 to {MAX_PREDICTORS})', size(X,2)); end\n\
        keep = ~isnan(y); result.dropped = sum(double(~keep)); result.records = find(keep); X = X(keep,:); y = y(keep);\n\
        if numel(y) < 3 || any(~isfinite(X(:))) || any(~isfinite(y)), error('Regression Learner: need at least three finite responses and finite predictors'); end\n\
        rng({seed}); partition = {partition};\n\
        predictions = NaN(size(y)); result.fold = zeros(size(y));\n\
        tic;\n\
        for fold = 1:partition.NumTestSets\n\
          trainRows = training(partition, fold); testRows = test(partition, fold);\n\
          [trainX, testX] = regressionLearnerPrepare(X(trainRows,:), X(testRows,:));\n\
          fitted = {fit};\n\
          predictions(testRows) = predict(fitted, testX); result.fold(testRows) = fold;\n\
        end\n\
        valid = ~isnan(predictions); actual = y(valid); predicted = predictions(valid); residual = actual - predicted;\n\
        if isempty(actual) || any(~isfinite(predicted)), error('Regression Learner: validation produced no finite predictions'); end\n\
        result.mse = mean(residual.^2); result.rmse = sqrt(result.mse); result.mae = mean(abs(residual));\n\
        total = sum((actual - mean(actual)).^2);\n\
        if total == 0, result.rsquared = NaN; else, result.rsquared = 1 - sum(residual.^2)/total; end\n\
        [allX, unused, mu, scale, center, coefficients] = regressionLearnerPrepare(X, X);\n\
        finalModel = {final_fit};\n\
        trained.RegressionModel = finalModel; trained.PredictorMean = mu; trained.PredictorScale = scale;\n\
        trained.PCACenter = center; trained.PCACoefficients = coefficients;\n\
        trained.predictFcn = @(newX) predict(finalModel, ((double(newX) - repmat(mu,size(newX,1),1))./repmat(scale,size(newX,1),1) - repmat(center,size(newX,1),1))*coefficients);\n\
        trained.NumObservations = numel(y); trained.Validation = {};\n\
        result.seconds = toc; result.actual = actual; result.predicted = predicted; result.residual = residual;\n\
        result.X = X(valid,:); result.records = result.records(valid);\n\
        end\n\
        function [A, B, mu, scale, center, coefficients] = regressionLearnerPrepare(A, B)\n\
        mu = zeros(1,size(A,2)); scale = ones(1,size(A,2));\n",
        quote(&spec.description())
    );
    if spec.standardize {
        code += "mu = mean(A,1); scale = std(A,0,1); scale(scale == 0) = 1; A = (A-repmat(mu,size(A,1),1))./repmat(scale,size(A,1),1); B = (B-repmat(mu,size(B,1),1))./repmat(scale,size(B,1),1);\n";
    }
    code += "center = zeros(1,size(A,2)); coefficients = eye(size(A,2));\n";
    if spec.pca {
        code += &format!(
            "[coefficients, scores, latent, tsquared, explained, center] = pca(A);\nif any(~isfinite(explained)), error('Regression Learner: PCA needs nonconstant predictors'); end\ncomponents = find(cumsum(explained) >= {variance} - 1e-10, 1);\nif isempty(components), components = size(coefficients,2); end\ncoefficients = coefficients(:,1:components); A = (A-repmat(center,size(A,1),1))*coefficients; B = (B-repmat(center,size(B,1),1))*coefficients;\n"
        );
    }
    code += "end\n";
    if model.preset == Preset::Robust {
        code = code
            .replace("predict(fitted, testX)", "fitted.predictFcn(testX)")
            .replace("predict(finalModel, ", "finalModel.predictFcn(");
        code += ROBUST_SCRIPT;
    }
    Ok(code)
}

// Tukey bisquare IRLS on leverage-adjusted residuals. QR/backslash solve
// weighted least squares; no matrix inverse. The independent NumPy oracle in
// tool/refs/regression_learner/reference.py checks every held-out prediction.
const ROBUST_SCRIPT: &str = "function model = regressionLearnerRobust(X, y, tune)\n\
    A = [ones(size(X,1),1), X];\n\
    if size(A,1) <= size(A,2), error('Regression Learner: robust linear needs positive residual degrees of freedom'); end\n\
     [Q,R] = qr(A,0); beta = R\\(Q' * y);\n\
    if rank(R) < size(A,2), error('Regression Learner: robust linear requires independent predictors'); end\n\
    leverage = sum(Q.^2,2); leverage(leverage > 1-eps) = 1-eps; adjustment = sqrt(1-leverage);\n\
    converged = false;\n\
    for iteration = 1:100\n\
      residual = (y-A*beta)./adjustment;\n\
      scale = median(abs(residual-median(residual)))/0.6744897501960817;\n\
      if scale <= eps*(1+norm(y)), converged = true; break; end\n\
      u = residual/(tune*scale); u(abs(u)>=1) = 1; weights = (1-u.^2).^2; active = weights > 0;\n\
      if sum(double(active)) <= size(A,2), error('Regression Learner: too few observations retain robust weights'); end\n\
      root = sqrt(weights(active)); weightedA = A(active,:).*repmat(root,1,size(A,2));\n\
      [weightedQ,weightedR] = qr(weightedA,0); next = weightedR\\(weightedQ'*(y(active).*root));\n\
      if norm(next-beta) <= 1e-8*(1+norm(beta)), beta = next; converged = true; break; end\n\
      beta = next;\n\
    end\n\
    if ~converged, error('Regression Learner: robust iteration did not converge in 100 steps'); end\n\
    model.Beta = beta; model.NumObservations = numel(y);\n\
    model.predictFcn = @(newX) [ones(size(newX,1),1), double(newX)]*beta;\n\
    end\n";

#[derive(Clone, Debug)]
pub struct FitResult {
    pub folds: Vec<f64>,
    pub rmse: f64,
    pub rsquared: f64,
    pub mse: f64,
    pub mae: f64,
    pub seconds: f64,
    pub dropped: usize,
    pub actual: Vec<f64>,
    pub predicted: Vec<f64>,
    pub residual: Vec<f64>,
    pub records: Vec<f64>,
    pub x: Vec<Vec<f64>>,
}
fn numbers(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Regression Learner: missing result {name}")),
    }
}
impl FitResult {
    pub fn from_outputs(outputs: &Outputs) -> Result<Self, String> {
        let metrics = numbers(outputs, "rl_metrics")?;
        if metrics.len() != 6 {
            return Err("Regression Learner: incomplete metrics".into());
        }
        let x = match outputs.get("rl_x") {
            Some(Array::Numeric(m)) => (0..m.cols())
                .map(|i| m.data()[i * m.rows()..(i + 1) * m.rows()].to_vec())
                .collect(),
            _ => return Err("Regression Learner: missing predictors".into()),
        };
        Ok(Self {
            folds: numbers(outputs, "rl_fold")?,
            rmse: metrics[0],
            rsquared: metrics[1],
            mse: metrics[2],
            mae: metrics[3],
            seconds: metrics[4],
            dropped: metrics[5] as usize,
            actual: numbers(outputs, "rl_actual")?,
            predicted: numbers(outputs, "rl_predicted")?,
            residual: numbers(outputs, "rl_residual")?,
            records: numbers(outputs, "rl_records")?,
            x,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Models,
    Response,
    PredictedActual,
    Residuals,
}
impl View {
    pub const ALL: [Self; 4] = [
        Self::Models,
        Self::Response,
        Self::PredictedActual,
        Self::Residuals,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Models => "Models",
            Self::Response => "Response",
            Self::PredictedActual => "Predicted vs actual",
            Self::Residuals => "Residuals",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Rmse,
    Rsquared,
    Mae,
    Time,
    Status,
}

#[derive(Clone)]
pub struct ModelEntry {
    pub model: ModelSpec,
    pub result: Option<FitResult>,
    pub status: String,
    trained_spec: Option<Spec>,
    trained_model: Option<ModelSpec>,
    trained_inputs: BTreeMap<String, Array>,
}
impl ModelEntry {
    pub fn new(preset: Preset) -> Self {
        Self {
            model: ModelSpec::new(preset),
            result: None,
            status: "Not trained".into(),
            trained_spec: None,
            trained_model: None,
            trained_inputs: BTreeMap::new(),
        }
    }
}
struct Job {
    index: usize,
    spec: Spec,
    model: ModelSpec,
    inputs: BTreeMap<String, Array>,
    request: Request,
}
pub struct RegressionLearner {
    pub spec: Spec,
    pub models: Vec<ModelEntry>,
    pub selected: usize,
    pub view: View,
    pub sort: Sort,
    pub descending: bool,
    pub response_axis: usize,
    pub residual_axis: usize,
    pub model_name: String,
    pub export_predictions: bool,
    pub prediction_name: String,
    pub message: String,
    pub error: bool,
    preset: Preset,
    queue: VecDeque<Job>,
    active: Option<Job>,
    runner: Runner,
    serial: u64,
}
impl Default for RegressionLearner {
    fn default() -> Self {
        Self {
            spec: Spec::default(),
            models: vec![ModelEntry::new(Preset::Linear)],
            selected: 0,
            view: View::Models,
            sort: Sort::Rmse,
            descending: false,
            response_axis: 0,
            residual_axis: 0,
            model_name: "trainedModel".into(),
            export_predictions: false,
            prediction_name: "validationPredictions".into(),
            message: "Choose data and train a model".into(),
            error: false,
            preset: Preset::Linear,
            queue: VecDeque::new(),
            active: None,
            runner: Runner::default(),
            serial: 0,
        }
    }
}
fn probe(
    script: &str,
    ws: &Snapshot,
    model_name: &str,
    prediction_name: Option<&str>,
) -> (String, Vec<String>) {
    let result = super::unused_name("rl_result", |n| {
        ws.contains(n) || n == model_name || prediction_name == Some(n)
    });
    let code = format!(
        "rl_metrics = [{result}.rmse {result}.rsquared {result}.mse {result}.mae {result}.seconds {result}.dropped];\nrl_actual = {result}.actual; rl_predicted = {result}.predicted; rl_residual = {result}.residual; rl_records = {result}.records; rl_x = {result}.X; rl_fold = {result}.fold;\n"
    );
    let (body, functions) = super::split_local_functions(script);
    (
        format!("{body}{code}{functions}"),
        [
            "rl_metrics",
            "rl_actual",
            "rl_predicted",
            "rl_residual",
            "rl_records",
            "rl_x",
            "rl_fold",
        ]
        .map(str::to_owned)
        .to_vec(),
    )
}
impl RegressionLearner {
    pub fn running(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty() || self.runner.running()
    }
    pub fn add(&mut self, preset: Preset) {
        if self.models.len() < MAX_MODELS && !self.running() {
            self.models.push(ModelEntry::new(preset));
            self.selected = self.models.len() - 1;
        }
    }
    pub fn train(&mut self, ws: &Snapshot, all: bool) {
        if self.running() {
            return;
        }
        self.error = false;
        let dependencies = self.spec.inputs();
        let inputs: BTreeMap<_, _> = ws
            .variables()
            .filter(|(name, _)| dependencies.iter().any(|n| n == name))
            .map(|(n, a)| (n.to_owned(), a.clone()))
            .collect();
        let ids: Vec<usize> = if all {
            (0..self.models.len()).collect()
        } else {
            vec![self.selected]
        };
        let prediction_name = self
            .export_predictions
            .then_some(self.prediction_name.as_str());
        for index in ids {
            let Some(entry) = self.models.get_mut(index) else {
                continue;
            };
            match script_text(
                &self.spec,
                &entry.model,
                ws,
                &self.model_name,
                prediction_name,
            ) {
                Ok(script) => {
                    self.serial += 1;
                    let (code, outputs) = probe(&script, ws, &self.model_name, prediction_name);
                    entry.status = "Queued".into();
                    self.queue.push_back(Job {
                        index,
                        spec: self.spec.clone(),
                        model: entry.model.clone(),
                        inputs: inputs.clone(),
                        request: Request {
                            key: self.serial,
                            inputs: inputs.iter().map(|(n, a)| (n.clone(), a.clone())).collect(),
                            code,
                            outputs,
                        },
                    });
                }
                Err(error) => {
                    entry.status = error.clone();
                    self.message = error;
                    self.error = true;
                }
            }
        }
        self.runner.set_folder(ws.folder());
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.runner.poll(ctx);
        if self.active.is_some()
            && let Some(result) = self.runner.result()
        {
            let job = self.active.take().unwrap();
            let result = result
                .as_ref()
                .map_err(|error| {
                    error
                        .split_once("Regression Learner: ")
                        .map_or(error.as_str(), |(_, message)| message)
                        .to_owned()
                })
                .and_then(FitResult::from_outputs);
            let entry = &mut self.models[job.index];
            match result {
                Ok(result) => {
                    let dropped = result.dropped;
                    let retained = result.folds.len();
                    entry.result = Some(result);
                    entry.trained_spec = Some(job.spec);
                    entry.trained_model = Some(job.model);
                    entry.trained_inputs = job.inputs;
                    entry.status =
                        format!("Trained · {retained} rows · {dropped} missing responses dropped");
                    if !self.error {
                        self.message = entry.status.clone();
                    }
                }
                Err(error) => {
                    entry.status = error.clone();
                    self.message = error;
                    self.error = true;
                }
            }
            self.runner.stop();
        }
        if self.active.is_none()
            && let Some(mut job) = self.queue.pop_front()
        {
            self.models[job.index].status = "Training".into();
            let request = std::mem::replace(
                &mut job.request,
                Request {
                    key: 0,
                    inputs: Vec::new(),
                    code: String::new(),
                    outputs: Vec::new(),
                },
            );
            self.runner.submit(ctx, request);
            self.active = Some(job);
        }
    }
    pub fn stop(&mut self) {
        if let Some(job) = self.active.take() {
            self.models[job.index].status = "Cancelled".into();
        }
        for job in self.queue.drain(..) {
            self.models[job.index].status = "Cancelled".into();
        }
        self.runner.stop();
        self.message = "Training cancelled".into();
        self.error = false;
    }
    pub fn order(&self) -> Vec<usize> {
        let mut order: Vec<_> = (0..self.models.len()).collect();
        order.sort_by(|a, b| {
            let a = &self.models[*a];
            let b = &self.models[*b];
            let metric = |e: &ModelEntry| {
                e.result
                    .as_ref()
                    .map(|r| match self.sort {
                        Sort::Rmse => r.rmse,
                        Sort::Rsquared => -r.rsquared,
                        Sort::Mae => r.mae,
                        Sort::Time => r.seconds,
                        _ => 0.,
                    })
                    .filter(|v| v.is_finite())
                    .unwrap_or(f64::INFINITY)
            };
            let cmp = match self.sort {
                Sort::Name => a.model.preset.label().cmp(b.model.preset.label()),
                Sort::Status => a.status.cmp(&b.status),
                _ => metric(a).total_cmp(&metric(b)),
            };
            if self.descending { cmp.reverse() } else { cmp }
        });
        order
    }
    fn selected_result(&self) -> Option<&FitResult> {
        self.models.get(self.selected)?.result.as_ref()
    }
    fn export_current(&self, ws: &Snapshot) -> Result<(), String> {
        let entry = self
            .models
            .get(self.selected)
            .ok_or("Regression Learner: select a model")?;
        if entry.result.is_none() || entry.trained_spec.is_none() || entry.trained_model.is_none() {
            return Err("Regression Learner: train the selected model first".into());
        }
        for (name, old) in &entry.trained_inputs {
            if !ws.get(name).is_some_and(|now| now.same_snapshot(old)) {
                return Err(format!(
                    "Regression Learner: {name} changed; retrain before exporting"
                ));
            }
        }
        Ok(())
    }
}

impl App for RegressionLearner {
    fn kind(&self) -> AppKind {
        AppKind::RegressionLearner
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        self.export_current(ws)?;
        let entry = &self.models[self.selected];
        script_text(
            entry.trained_spec.as_ref().unwrap(),
            entry.trained_model.as_ref().unwrap(),
            ws,
            &self.model_name,
            self.export_predictions
                .then_some(self.prediction_name.as_str()),
        )
    }
    fn inputs(&self) -> Vec<String> {
        self.models
            .get(self.selected)
            .and_then(|m| m.trained_spec.as_ref())
            .unwrap_or(&self.spec)
            .inputs()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(script) = self.script(ws) else {
            return (String::new(), Vec::new());
        };
        let (code, outputs) = probe(
            &script,
            ws,
            &self.model_name,
            self.export_predictions
                .then_some(self.prediction_name.as_str()),
        );
        let (body, _) = super::split_local_functions(&code);
        let (original, _) = super::split_local_functions(&script);
        (body[original.len()..].into(), outputs)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let result = FitResult::from_outputs(outputs)?;
        if let Some(entry) = self.models.get_mut(self.selected) {
            entry.result = Some(result);
        }
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let running = self.running();
        ui.add_enabled_ui(!running, |ui| {
            ui.checkbox(&mut self.spec.use_table, "Use workspace table");
            if self.spec.use_table {
                picker(
                    ui,
                    "rl_table",
                    "Table",
                    &mut self.spec.table,
                    &super::curve_fitter::tables(ws)
                        .into_iter()
                        .map(|(n, _)| n)
                        .collect::<Vec<_>>(),
                );
                let columns = super::curve_fitter::table_columns(ws, &self.spec.table)
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect::<Vec<_>>();
                picker(
                    ui,
                    "rl_response",
                    "Response",
                    &mut self.spec.response,
                    &columns,
                );
                self.spec
                    .predictors
                    .retain(|n| columns.contains(n) && n != &self.spec.response);
                field(ui, "Predictors", |ui| {
                    for name in columns.iter().filter(|n| **n != self.spec.response) {
                        let mut checked = self.spec.predictors.contains(name);
                        if ui.checkbox(&mut checked, name).changed() {
                            if checked {
                                self.spec.predictors.push(name.clone());
                            } else {
                                self.spec.predictors.retain(|n| n != name);
                            }
                        }
                    }
                });
            } else {
                text_field(ui, "Predictor matrix X", &mut self.spec.x);
                text_field(ui, "Response vector y", &mut self.spec.y);
            }
            field(ui, "Validation", |ui| {
                egui::ComboBox::from_id_salt("rl_validation")
                    .selected_text(self.spec.validation.label())
                    .show_ui(ui, |ui| {
                        for v in Validation::ALL {
                            ui.selectable_value(&mut self.spec.validation, v, v.label());
                        }
                    });
            });
            match self.spec.validation {
                Validation::KFold => text_field(ui, "Folds", &mut self.spec.folds),
                Validation::Holdout => text_field(ui, "Holdout percent", &mut self.spec.holdout),
                _ => {}
            }
            text_field(ui, "Random seed", &mut self.spec.seed);
            ui.checkbox(&mut self.spec.standardize, "Standardize predictors");
            ui.checkbox(&mut self.spec.pca, "Principal components");
            if self.spec.pca {
                text_field(ui, "Explained variance percent", &mut self.spec.variance);
            }
            ui.add_space(crate::tokens::S);
            field(ui, "Model preset", |ui| {
                egui::ComboBox::from_id_salt("rl_preset")
                    .selected_text(self.preset.label())
                    .show_ui(ui, |ui| {
                        for p in Preset::ALL {
                            ui.selectable_value(&mut self.preset, p, p.label());
                        }
                    });
            });
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
                .add_enabled(
                    !self.models.is_empty(),
                    egui::Button::new("Remove selected"),
                )
                .clicked()
            {
                self.models.remove(self.selected);
                self.selected = self.selected.min(self.models.len().saturating_sub(1));
            }
            if let Some(entry) = self.models.get_mut(self.selected) {
                let p = entry.model.preset;
                crate::tokens::helper(ui, p.label());
                if p == Preset::Robust {
                    text_field(ui, "Bisquare tuning constant", &mut entry.model.robust_tune);
                }
                if p.tree() {
                    text_field(ui, "Minimum leaf size", &mut entry.model.leaf);
                }
                if matches!(p, Preset::Bagged | Preset::Boosted) {
                    text_field(ui, "Learning cycles", &mut entry.model.cycles);
                    text_field(ui, "Learning rate", &mut entry.model.rate);
                }
                if p.svm() {
                    text_field(ui, "Box constraint", &mut entry.model.box_constraint);
                    text_field(ui, "Epsilon", &mut entry.model.epsilon);
                    text_field(ui, "Kernel scale", &mut entry.model.scale);
                }
                if p.gp() {
                    ui.checkbox(&mut entry.model.gp_linear_basis, "Linear GP mean");
                    text_field(ui, "Initial noise sigma", &mut entry.model.sigma);
                    text_field(ui, "Initial length scale", &mut entry.model.length_scale);
                    text_field(ui, "Initial signal scale", &mut entry.model.signal_scale);
                    if p == Preset::RationalQuadraticGp {
                        text_field(ui, "Initial shape", &mut entry.model.shape);
                    }
                }
                if matches!(p, Preset::LinearLeastSquares | Preset::LinearHighSvm) {
                    text_field(ui, "Lambda", &mut entry.model.lambda);
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!self.models.is_empty(), egui::Button::new("Train selected"))
                    .clicked()
                {
                    self.train(ws, false);
                }
                if ui
                    .add_enabled(!self.models.is_empty(), egui::Button::new("Train all"))
                    .clicked()
                {
                    self.train(ws, true);
                }
            });
        });
        if ui.add_enabled(running, egui::Button::new("Stop")).clicked() {
            self.stop();
        }
        crate::tokens::helper(
            ui,
            "Up to 32 models, 20000 rows and 128 predictors. Exact GP supports 512 rows. Preprocessing is fitted within each fold.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "Model comparison"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for view in View::ALL {
                ui.selectable_value(&mut self.view, view, view.label());
            }
        });
        ui.add_space(crate::tokens::S);
        if self.view == View::Models {
            let best = self
                .models
                .iter()
                .filter_map(|m| m.result.as_ref().map(|r| r.rmse))
                .filter(|v| v.is_finite())
                .min_by(f64::total_cmp);
            egui::ScrollArea::both()
                .id_salt("rl_models")
                .show_themed(ui, |ui| {
                    egui::Grid::new("rl_model_grid")
                        .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                        .striped(true)
                        .show(ui, |ui| {
                            for (sort, label) in [
                                (Sort::Name, "Model"),
                                (Sort::Rmse, "RMSE"),
                                (Sort::Rsquared, "R-squared"),
                                (Sort::Mae, "MAE"),
                                (Sort::Time, "Seconds"),
                                (Sort::Status, "Status"),
                            ] {
                                if ui.selectable_label(self.sort == sort, label).clicked() {
                                    if self.sort == sort {
                                        self.descending = !self.descending;
                                    } else {
                                        self.sort = sort;
                                        self.descending = false;
                                    }
                                }
                            }
                            ui.end_row();
                            for index in self.order() {
                                let entry = &self.models[index];
                                let best_mark = if entry
                                    .result
                                    .as_ref()
                                    .is_some_and(|r| Some(r.rmse) == best)
                                {
                                    " *"
                                } else {
                                    ""
                                };
                                ui.selectable_value(
                                    &mut self.selected,
                                    index,
                                    format!("{}{best_mark}", entry.model.preset.label()),
                                );
                                for value in entry
                                    .result
                                    .as_ref()
                                    .map(|r| {
                                        [
                                            short(r.rmse),
                                            short(r.rsquared),
                                            short(r.mae),
                                            short(r.seconds),
                                        ]
                                    })
                                    .unwrap_or_else(|| std::array::from_fn(|_| "—".into()))
                                {
                                    ui.label(value);
                                }
                                ui.label(&entry.status);
                                ui.end_row();
                            }
                        });
                });
        } else {
            let entry = self.models.get(self.selected);
            let Some(result) = entry.and_then(|entry| entry.result.as_ref()) else {
                crate::tokens::helper(ui, "Train a model to see validation predictions.");
                return;
            };
            let spec = entry.and_then(|entry| entry.trained_spec.as_ref());
            let labels: Vec<_> = (0..result.x.len())
                .map(|i| {
                    spec.filter(|s| s.use_table)
                        .and_then(|s| s.predictors.get(i))
                        .cloned()
                        .unwrap_or_else(|| format!("Predictor {}", i + 1))
                })
                .collect();
            let mut choices = if self.view == View::Response {
                vec!["Record number".to_owned()]
            } else {
                vec!["Predicted response".to_owned(), "True response".into()]
            };
            choices.extend(labels);
            let axis = if self.view == View::Response {
                &mut self.response_axis
            } else {
                &mut self.residual_axis
            };
            *axis = (*axis).min(choices.len() - 1);
            if self.view != View::PredictedActual {
                egui::ComboBox::from_id_salt("rl_plot_axis")
                    .selected_text(&choices[*axis])
                    .show_ui(ui, |ui| {
                        for (i, label) in choices.iter().enumerate() {
                            ui.selectable_value(axis, i, label);
                        }
                    });
            }
            let x = match self.view {
                View::Response => {
                    if *axis == 0 {
                        &result.records
                    } else {
                        &result.x[*axis - 1]
                    }
                }
                View::Residuals => match *axis {
                    0 => &result.predicted,
                    1 => &result.actual,
                    i => &result.x[i - 2],
                },
                _ => &result.actual,
            };
            let y = if self.view == View::Residuals {
                &result.residual
            } else {
                &result.predicted
            };
            let p = crate::tokens::palette(ui.ctx());
            let points =
                |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(a, b)| [*a, *b]).collect::<Vec<_>>();
            let xlabel = if self.view == View::PredictedActual {
                "Actual"
            } else {
                &choices[*axis]
            };
            let ylabel = if self.view == View::Residuals {
                "Residual"
            } else {
                "Response"
            };
            egui_plot::Plot::new("rl_validation_plot")
                .height(ui.available_height().max(160.))
                .allow_scroll(false)
                .show_background(false)
                .x_axis_label(xlabel)
                .y_axis_label(ylabel)
                .legend(egui_plot::Legend::default())
                .show(ui, |plot| {
                    plot.points(
                        egui_plot::Points::new(
                            if self.view == View::Residuals {
                                "Residual"
                            } else {
                                "Predicted"
                            },
                            points(x, y),
                        )
                        .color(p.accent)
                        .radius(2.5),
                    );
                    if self.view == View::Response {
                        plot.points(
                            egui_plot::Points::new("True", points(x, &result.actual))
                                .color(p.text_muted)
                                .radius(2.5),
                        );
                    }
                    if self.view == View::PredictedActual {
                        let lo = result.actual.iter().copied().fold(f64::INFINITY, f64::min);
                        let hi = result
                            .actual
                            .iter()
                            .copied()
                            .fold(f64::NEG_INFINITY, f64::max);
                        plot.line(
                            egui_plot::Line::new("y = x", vec![[lo, lo], [hi, hi]])
                                .color(p.text_muted),
                        );
                    }
                    if self.view == View::Residuals {
                        plot.hline(egui_plot::HLine::new("Zero", 0.).color(p.text_muted));
                    }
                });
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(result) = self.selected_result() else {
            return Vec::new();
        };
        let entry = &self.models[self.selected];
        vec![
            (
                "Model",
                entry
                    .trained_model
                    .as_ref()
                    .unwrap_or(&entry.model)
                    .preset
                    .label()
                    .into(),
            ),
            ("RMSE", short(result.rmse)),
            ("R-squared", short(result.rsquared)),
            ("MSE", short(result.mse)),
            ("MAE", short(result.mae)),
            (
                "Validation",
                entry
                    .trained_spec
                    .as_ref()
                    .map_or_else(String::new, Spec::description),
            ),
            (
                "Hyperparameters",
                entry
                    .trained_model
                    .as_ref()
                    .unwrap_or(&entry.model)
                    .fit_call("X", "y")
                    .unwrap_or_default(),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Trained model", &mut self.model_name, ws);
        ui.checkbox(
            &mut self.export_predictions,
            "Export validation predictions",
        );
        if self.export_predictions {
            name_field(ui, "Predictions", &mut self.prediction_name, ws);
        }
        crate::tokens::helper(
            ui,
            "Exports a model fitted on every retained row, with a predictFcn that applies its preprocessing.",
        );
        super::expression_message(ui, self.script(ws).map(|_| ()));
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        let mut names = vec![self.model_name.clone()];
        if self.export_predictions {
            names.push(self.prediction_name.clone());
        }
        names
    }
    fn summary(&self) -> Option<String> {
        Some(self.message.clone())
    }
}
fn text_field(ui: &mut egui::Ui, label: &str, text: &mut String) {
    field(ui, label, |ui| {
        ui.add(
            crate::tokens::input(egui::TextEdit::singleline(text).char_limit(4000))
                .desired_width(ui.available_width()),
        );
        super::expression_message(ui, expression(text).map(|_| ()));
    });
}
fn picker(ui: &mut egui::Ui, id: &str, label: &str, value: &mut String, options: &[String]) {
    field(ui, label, |ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(if value.is_empty() {
                "Choose a variable"
            } else {
                value.as_str()
            })
            .show_ui(ui, |ui| {
                for name in options {
                    ui.selectable_value(value, name.clone(), name);
                }
            });
    });
}
pub(crate) fn show(
    host: &mut Host<RegressionLearner>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.app.poll(ui.ctx());
    let ready = !busy && !host.saving && !host.app.running() && host.app.script(ws).is_ok();
    let mut clicked = None;
    super::three_areas(
        ui,
        AppKind::RegressionLearner,
        "Model comparison",
        |ui, area| match area {
            super::Area::Inputs => host.app.inputs_ui(ui, ws),
            super::Area::Main => host.app.main_ui(ui),
            super::Area::Output => {
                super::result_block(ui, &host.app.results());
                ui.add_space(crate::tokens::M);
                host.app.names_ui(ui, ws);
                clicked = super::output_buttons(ui, ready);
                if !host.message.is_empty() {
                    crate::tokens::helper(ui, &host.message);
                }
            }
        },
    );
    *action = match clicked {
        Some(super::OutputClick::Export) => host.export(ws),
        Some(super::OutputClick::Script) => host.generate(ws, super::now()),
        None => action.take(),
    };
}
pub(crate) fn status(host: &Host<RegressionLearner>) -> (String, AppStatus) {
    let app = &host.app;
    if app.error {
        return (app.message.clone(), AppStatus::Error);
    }
    if let Some(job) = &app.active {
        return (
            format!(
                "{}: training · {} queued",
                job.model.preset.label(),
                app.queue.len()
            ),
            AppStatus::Plain,
        );
    }
    (
        app.message.clone(),
        if app.selected_result().is_some() {
            AppStatus::Success
        } else {
            AppStatus::Plain
        },
    )
}
