//! Desktop-independent shallow-network training.
//!
//! `train(&network, &x, &t, opts, &mut |EpochReport| ControlFlow::Continue(()))`
//! returns `(Network, TrainingRecord)`. Break returns current weights and "User stop.";
//! cancel is an error and never mutates the caller. Options supply the RNG for initial
//! configuration/random division (the interpreter supplies its global stream).
//!
//! Extension: add train_NAME.rs with a Default struct implementing TrainFn, then one
//! registry! entry below. Defaults, workspace admission, step and extra record states
//! are dispatched through the trait; no Engine methods or epoch-loop edits needed.
//! Objective supplies analytic gradients and optional half Gauss-Newton curvature.
//! LM: Hagan & Menhaj (1994). SCG: Moller (1993). Choices and independent references:
//! tool/refs/nnet/train.py. Public contracts: MathWorks train/trainlm/trainscg pages.
use super::{divide::Division, network::Network};
use crate::{Matrix, Result};
use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::{Arc, atomic::AtomicBool},
};
#[path = "train_objective.rs"]
mod objective;
pub use objective::{Evaluation, Objective};
pub type Parameters = BTreeMap<String, f64>;
pub type CommandLineSink<'a> = &'a mut dyn FnMut(&str) -> Result<()>;

pub trait TrainFn {
    fn defaults(&self) -> Parameters;
    /// String parameters (for example trainbfg searchFcn) stay out of numeric state.
    fn string_defaults(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    /// Override when an algorithm has an evolving objective (Bayesian regularization).
    fn evaluate(&self, net: &Network, objective: &Objective<'_>) -> Result<Evaluation> {
        objective.evaluate(net, false)
    }
    fn workspace(&self, weights: usize) -> usize {
        weights.saturating_mul(32)
    }
    fn initialize(&mut self, _p: &Parameters, _objective: &Objective<'_>) -> Result<()> {
        Ok(())
    }
    /// One attempted epoch. A rejected step retains the current network.
    fn step(
        &mut self,
        net: &mut Network,
        objective: &Objective<'_>,
        current: &Evaluation,
        p: &Parameters,
    ) -> Result<()>;
    fn extra_fields(&self) -> Vec<(&'static str, f64)> {
        vec![]
    }
    fn stop(&self, _p: &Parameters) -> Option<&'static str> {
        None
    }
}
macro_rules! registry {
    ($($module:ident => $kind:ident ($path:literal)),* $(,)?) => {
        $(#[path = $path] mod $module;)*
        pub fn algorithm(name: &str) -> Result<Box<dyn TrainFn>> {
            match name {
                $(stringify!($module) => Ok(Box::new($module::$kind::default())),)*
                _ => Err(format!("train: trainFcn '{name}' is not implemented yet")),
            }
        }
    };
}
registry! {
    trainlm => Lm ("trainlm.rs"), trainscg => Scg ("trainscg.rs"),
    trainbr => Br ("trainbr.rs"),
    traingd => Gd ("traingd.rs"),
    traingdm => Gdm ("traingdm.rs"),
    traingdx => Gdx ("traingdx.rs"),
    trainrp => Rp ("trainrp.rs"),
    trainbfg => Bfg ("trainbfg.rs"),
}

pub fn common_defaults(min_grad: f64) -> Parameters {
    [
        ("epochs", 1000.),
        ("goal", 0.),
        ("max_fail", 6.),
        ("min_grad", min_grad),
        ("show", 25.),
        ("showCommandLine", 0.),
        ("showWindow", 1.),
        ("time", f64::INFINITY),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v))
    .collect()
}
pub fn defaults(name: &str) -> Result<Parameters> {
    Ok(algorithm(name)?.defaults())
}

pub struct TrainOptions<'a> {
    pub train_fcn: String,
    pub perform_fcn: String,
    /// Overrides of algorithm defaults.
    pub parameters: Parameters,
    pub string_parameters: BTreeMap<String, String>,
    pub division: Division,
    pub error_weights: Option<Matrix>,
    pub regularization: f64,
    pub normalization: String,
    pub cancel: Arc<AtomicBool>,
    pub memory_bytes: usize,
    pub output_bytes: usize,
    /// Optional host console sink; defaults to stdout. Apps normally leave showCommandLine false.
    pub command_line: Option<CommandLineSink<'a>>,
    pub uniform: Option<&'a mut dyn FnMut() -> f64>,
}
impl Default for TrainOptions<'_> {
    fn default() -> Self {
        Self {
            train_fcn: "trainlm".into(),
            perform_fcn: "mse".into(),
            parameters: Parameters::new(),
            string_parameters: BTreeMap::new(),
            division: Division::All,
            error_weights: None,
            regularization: 0.,
            normalization: "none".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            memory_bytes: 256 * 1024 * 1024,
            output_bytes: 2 * 1024 * 1024,
            command_line: None,
            uniform: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct EpochReport {
    pub epoch: usize,
    pub time: f64,
    pub perf: f64,
    pub vperf: f64,
    pub tperf: f64,
    pub gradient: f64,
    pub val_fail: usize,
    pub extra: BTreeMap<String, f64>,
}
#[derive(Clone, Debug)]
pub struct TrainingRecord {
    pub train_fcn: String,
    pub train_param: Parameters,
    pub train_param_strings: BTreeMap<String, String>,
    pub perform_fcn: String,
    pub division: Division,
    pub train_ind: Vec<usize>,
    pub val_ind: Vec<usize>,
    pub test_ind: Vec<usize>,
    pub stop: String,
    pub num_epochs: usize,
    pub best_epoch: usize,
    pub best_perf: f64,
    pub best_vperf: f64,
    pub best_tperf: f64,
    pub epochs: Vec<EpochReport>,
}
fn validate(p: &Parameters) -> Result<()> {
    for (k, &v) in p {
        if k == "show" && v.is_nan() {
            continue;
        }
        if v.is_nan() || v < 0. || (k != "time" && !v.is_finite()) {
            return Err(format!("trainParam.{k}: expected nonnegative finite value"));
        }
        if matches!(k.as_str(), "epochs" | "max_fail" | "show") && v.fract() != 0. {
            return Err(format!("trainParam.{k}: expected integer"));
        }
        if matches!(k.as_str(), "showCommandLine" | "showWindow") && v != 0. && v != 1. {
            return Err(format!("trainParam.{k}: expected logical value"));
        }
    }
    if p["epochs"] > 1_000_000. {
        return Err("train: epoch record limit exceeded".into());
    }
    Ok(())
}
pub fn train(
    network: &Network,
    x: &Matrix,
    t: &Matrix,
    mut opts: TrainOptions<'_>,
    progress: &mut dyn FnMut(EpochReport) -> ControlFlow<()>,
) -> Result<(Network, TrainingRecord)> {
    crate::dsp::check_cancel(&opts.cancel)?;
    let mut method = algorithm(&opts.train_fcn)?;
    let mut p = method.defaults();
    for (k, v) in &opts.parameters {
        if !p.contains_key(k) {
            return Err(format!("train: unknown trainParam.{k}"));
        }
        p.insert(k.clone(), *v);
    }
    validate(&p)?;
    let mut strings = method.string_defaults();
    for (key, value) in &opts.string_parameters {
        if !strings.contains_key(key) {
            return Err(format!("train: unknown trainParam.{key}"));
        }
        strings.insert(key.clone(), value.clone());
    }
    opts.string_parameters = strings;
    if x.is_nd()
        || t.is_nd()
        || x.cols() != t.cols()
        || x.cols() == 0
        || t.rows() == 0
        || x.data().iter().chain(t.data()).any(|v| !v.is_finite())
    {
        return Err(
            "train: finite nonempty input/target matrices with equal sample counts required".into(),
        );
    }
    if (!network.configured || matches!(opts.division, Division::Random(_)))
        && opts.uniform.is_none()
    {
        return Err("train: supply opts.uniform for initialization or random division".into());
    }
    let mut fallback = || 0.5;
    let uniform = opts.uniform.take().unwrap_or(&mut fallback);
    let mut net = network.clone();
    // Admission before configure allocates weights. Processing can only remove rows.
    if !net.configured {
        let mut upper = net.clone();
        upper.input.size = x.rows();
        upper.input.process_settings.clear();
        upper.layers.last_mut().ok_or("train: empty network")?.size = t.rows();
        let weights = upper.check_size()?;
        admit(
            &upper,
            x,
            t,
            weights,
            &p,
            method.as_ref(),
            opts.memory_bytes,
        )?;
        net.configure(x, t, &opts.cancel)?;
        net.init(&mut *uniform, &opts.cancel)?;
    }
    if net.input.size != x.rows() || net.output.size != t.rows() {
        return Err("train: data dimensions do not match configured network".into());
    }
    admit(
        &net,
        x,
        t,
        net.num_weight_elements(),
        &p,
        method.as_ref(),
        opts.memory_bytes,
    )?;
    let [train_ind, val_ind, test_ind] = opts.division.split(x.cols(), uniform, &opts.cancel)?;
    if train_ind.is_empty() {
        return Err("train: training division is empty".into());
    }
    let mut console = opts.command_line.take();
    let mut output_bytes = 0usize;
    let objective = Objective::new(&net, x, t, &train_ind, &opts)?;
    method.initialize(&p, &objective)?;
    let mut tr = TrainingRecord {
        train_fcn: opts.train_fcn.clone(),
        train_param: p.clone(),
        train_param_strings: opts.string_parameters.clone(),
        perform_fcn: opts.perform_fcn.clone(),
        division: opts.division.clone(),
        train_ind: train_ind.clone(),
        val_ind: val_ind.clone(),
        test_ind: test_ind.clone(),
        stop: String::new(),
        num_epochs: 0,
        best_epoch: 0,
        best_perf: f64::INFINITY,
        best_vperf: f64::NAN,
        best_tperf: f64::NAN,
        epochs: Vec::new(),
    };
    let validation = !val_ind.is_empty() && p["max_fail"] > 0.;
    let mut best = net.clone();
    let mut best_score = f64::INFINITY;
    let mut failures = 0;
    let start = objective.started;
    for epoch in 0..=p["epochs"] as usize {
        crate::dsp::check_cancel(&opts.cancel)?;
        let current = method.evaluate(&net, &objective)?;
        let vperf = objective.performance(&net, &val_ind)?;
        let tperf = objective.performance(&net, &test_ind)?;
        let score = if validation { vperf } else { current.perf };
        if epoch == 0 || score < best_score {
            best_score = score;
            best = net.clone();
            failures = 0;
            tr.best_epoch = epoch;
            tr.best_perf = current.perf;
            tr.best_vperf = vperf;
            tr.best_tperf = tperf;
        } else if validation {
            failures += 1;
        }
        let report = EpochReport {
            epoch,
            time: start.elapsed().as_secs_f64(),
            perf: current.perf,
            vperf,
            tperf,
            gradient: dot(&current.gradient, &current.gradient).sqrt(),
            val_fail: failures,
            extra: method
                .extra_fields()
                .into_iter()
                .map(|(k, v)| (k.into(), v))
                .collect(),
        };
        let stop = if current.perf <= p["goal"] {
            Some("Performance goal met.")
        } else if epoch == p["epochs"] as usize {
            Some("Maximum epoch reached.")
        } else if report.time >= p["time"] {
            Some("Maximum time elapsed.")
        } else if report.gradient <= p["min_grad"] {
            Some("Minimum gradient reached.")
        } else if let Some(s) = method.stop(&p) {
            Some(s)
        } else if validation && failures >= p["max_fail"] as usize {
            Some("Validation stop.")
        } else {
            None
        };
        if p["showCommandLine"] != 0.
            && p["show"].is_finite()
            && p["show"] > 0.
            && (epoch.is_multiple_of(p["show"] as usize) || stop.is_some())
        {
            let line = format!(
                "{}: Epoch {}/{}, {} {}/{}, Gradient {}/{}\n",
                opts.train_fcn.to_uppercase(),
                epoch,
                p["epochs"],
                opts.perform_fcn.to_uppercase(),
                current.perf,
                p["goal"],
                report.gradient,
                p["min_grad"]
            );
            output_bytes = output_bytes.saturating_add(line.len());
            if output_bytes > opts.output_bytes {
                return Err("train: progress output limit exceeded".into());
            }
            if let Some(sink) = &mut console {
                sink(&line)?;
            } else {
                print!("{line}");
            }
        }
        let user_stop = progress(report.clone()).is_break();
        crate::dsp::check_cancel(&opts.cancel)?;
        tr.epochs.push(report);
        tr.num_epochs = epoch;
        if user_stop {
            tr.stop = "User stop.".into();
            break;
        }
        if let Some(stop) = stop {
            tr.stop = stop.into();
            if validation {
                net = best;
            }
            break;
        }
        method.step(&mut net, &objective, &current, &p)?;
    }
    Ok((net, tr))
}
fn admit(
    net: &Network,
    x: &Matrix,
    t: &Matrix,
    w: usize,
    p: &Parameters,
    method: &dyn TrainFn,
    limit: usize,
) -> Result<()> {
    let units = net
        .layers
        .iter()
        .fold(0usize, |a, l| a.saturating_add(l.size));
    let bytes = units
        .saturating_mul(x.cols())
        .saturating_mul(64)
        .saturating_add(x.len().saturating_add(t.len()).saturating_mul(64))
        .saturating_add(w.saturating_mul(256))
        .saturating_add(method.workspace(w))
        .saturating_add((p["epochs"] as usize + 1).saturating_mul(512));
    if bytes > limit {
        Err("train: workspace memory limit exceeded".into())
    } else {
        Ok(())
    }
}
pub fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub fn trial(net: &Network, direction: &[f64], scale: f64) -> Result<Network> {
    let mut out = net.clone();
    out.setwb(
        &net.getwb()
            .iter()
            .zip(direction)
            .map(|(w, d)| w + scale * d)
            .collect::<Vec<_>>(),
    )?;
    Ok(out)
}
