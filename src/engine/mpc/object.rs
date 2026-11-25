//! MPC controller value object (`mpc`).
//!
//! Public specification: MathWorks `mpc`, `mpcprops`, and "MPC Controller Object"
//! properties documentation. Value semantics: copying `c2 = c` or mutating a
//! property replaces only the target binding; aliases keep their prior state.

use super::super::control_lti::{self, Lti};
use super::super::{Engine, Execution, Value};
use crate::syntax::PathStep;
use crate::{Array, CellArray, CharacterRow, Matrix, Result, StructArray};
use std::sync::Arc;

const MAX_HORIZON: usize = 10_000;
const MAX_ITERATIONS: usize = 1_000_000;

pub(in crate::engine) const PROPERTY_NAMES: &[&str] = &[
    "Ts",
    "PredictionHorizon",
    "ControlHorizon",
    "Weights",
    "ManipulatedVariables",
    "OutputVariables",
    "DisturbanceVariables",
    "Model",
    "Optimizer",
    "Notes",
    "UserData",
    "History",
];

#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) enum ControlHorizon {
    Scalar(usize),
    Blocking(Vec<usize>),
}

#[derive(Clone, Debug)]
pub(in crate::engine) struct MpcWeights {
    pub mv: Matrix,
    pub mv_rate: Matrix,
    pub ov: Matrix,
    pub ecr: f64,
}

#[derive(Clone, Debug)]
pub(in crate::engine) enum MvTarget {
    Scalar(f64),
    Vector(Matrix),
    Nominal,
}

#[derive(Clone, Debug)]
pub(in crate::engine) struct MvSpec {
    pub min: Matrix,
    pub max: Matrix,
    pub rate_min: Matrix,
    pub rate_max: Matrix,
    pub min_ecr: Matrix,
    pub max_ecr: Matrix,
    pub rate_min_ecr: Matrix,
    pub rate_max_ecr: Matrix,
    pub target: MvTarget,
    pub name: String,
    pub units: String,
    pub scale_factor: f64,
}

#[derive(Clone, Debug)]
pub(in crate::engine) struct OvSpec {
    pub min: Matrix,
    pub max: Matrix,
    pub min_ecr: Matrix,
    pub max_ecr: Matrix,
    pub name: String,
    pub units: String,
    pub scale_factor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct DvSpec {
    pub name: String,
    pub units: String,
    pub scale_factor: f64,
}

#[derive(Clone, Debug)]
pub(in crate::engine) struct MpcNominal {
    pub x: Matrix,
    pub u: Matrix,
    pub y: Matrix,
    pub dx: Matrix,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct MpcOptimizer {
    pub max_iterations: usize,
    pub constraint_tolerance: f64,
    pub algorithm: String,
}

#[derive(Clone)]
pub(in crate::engine) struct MpcObject {
    pub plant_array: Array,
    pub ad: Matrix,
    pub bd: Matrix,
    pub cd: Matrix,
    pub dd: Matrix,
    pub mv_idx: Vec<usize>,
    pub md_idx: Vec<usize>,
    pub ud_idx: Vec<usize>,
    pub mo_idx: Vec<usize>,
    pub uo_idx: Vec<usize>,
    pub ts: f64,
    pub prediction_horizon: usize,
    pub control_horizon: ControlHorizon,
    pub weights: MpcWeights,
    pub mv: Vec<MvSpec>,
    pub ov: Vec<OvSpec>,
    pub dv: Vec<DvSpec>,
    pub disturbance: Array,
    pub output_disturbance: Array,
    pub estimator: super::estimator::EstimatorMode,
    pub noise: Array,
    pub nominal: MpcNominal,
    pub optimizer: MpcOptimizer,
    pub notes: Array,
    pub user_data: Array,
    pub history: String,
}

impl MpcObject {
    pub fn nx(&self) -> usize {
        self.ad.rows()
    }
    pub fn nu(&self) -> usize {
        self.bd.cols()
    }
    pub fn ny(&self) -> usize {
        self.cd.rows()
    }
    pub fn n_mv(&self) -> usize {
        self.mv_idx.len()
    }
    pub fn n_ov(&self) -> usize {
        self.mo_idx.len() + self.uo_idx.len()
    }
    pub fn n_dv(&self) -> usize {
        self.md_idx.len() + self.ud_idx.len()
    }
    pub fn bytes(&self) -> usize {
        1024 + self.plant_array.retained_bytes()
            + (self.ad.len() + self.bd.len() + self.cd.len() + self.dd.len()) * 8
            + (self.weights.mv.len() + self.weights.mv_rate.len() + self.weights.ov.len()) * 8
            + (self.nominal.x.len()
                + self.nominal.u.len()
                + self.nominal.y.len()
                + self.nominal.dx.len())
                * 8
            + self.disturbance.retained_bytes()
            + self.noise.retained_bytes()
            + self.output_disturbance.retained_bytes()
            + match &self.estimator {
                super::estimator::EstimatorMode::Gains(l, m) => 8 * (l.len() + m.len()),
                _ => 0,
            }
            + self.notes.retained_bytes()
            + self.user_data.retained_bytes()
            + self.history.len()
    }
}

fn is_empty_arg(v: &Value) -> bool {
    matches!(v, Value::Matrix(m) if m.is_empty())
}

fn text_opt(e: &Engine, a: &Array) -> Option<String> {
    match a {
        Array::Character(c) if c.rows() <= 1 => c.to_utf8(&e.cancel).ok(),
        Array::Str(s) if s.len() == 1 => s.data()[0].as_deref().map(str::to_owned),
        _ => None,
    }
}

fn text_req(e: &Engine, a: &Array, prop: &str) -> Result<String> {
    text_opt(e, a).ok_or_else(|| format!("mpc: {prop} must be a character vector or string scalar"))
}

fn char_arr(e: &Engine, s: &str) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(s, &e.cancel)?))
}

fn empty_matrix() -> Array {
    Array::Numeric(Matrix::filled(0, 0, 0.).expect("0x0 matrix"))
}

fn struct_arr(e: &Engine, fields: Vec<(&str, Array)>) -> Result<Array> {
    let (names, slots) = fields
        .into_iter()
        .map(|(k, v)| (k.to_string(), Arc::new(v)))
        .unzip();
    Ok(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?))
}

fn real_matrix<'a>(a: &'a Array, name: &str) -> Result<&'a Matrix> {
    match a {
        Array::Numeric(m) if !m.is_nd() => Ok(m),
        _ => Err(format!("mpc: {name} must be a real double matrix")),
    }
}

fn real_scalar(a: &Array, name: &str) -> Result<f64> {
    let m = real_matrix(a, name)?;
    if m.len() != 1 {
        return Err(format!("mpc: {name} must be a scalar"));
    }
    Ok(m.data()[0])
}

fn positive_finite_scalar(a: &Array, name: &str) -> Result<f64> {
    let v = real_scalar(a, name)?;
    if !v.is_finite() || v <= 0. {
        return Err(format!("mpc: {name} must be a positive finite scalar"));
    }
    Ok(v)
}

fn positive_int_scalar(a: &Array, name: &str, max: usize) -> Result<usize> {
    let v = real_scalar(a, name)?;
    if !v.is_finite() || v < 1. || v.fract() != 0. || v > max as f64 {
        return Err(format!(
            "mpc: {name} must be a positive integer scalar (<= {max})"
        ));
    }
    Ok(v as usize)
}

fn parse_control_horizon(a: &Array, p: usize) -> Result<ControlHorizon> {
    let m = real_matrix(a, "ControlHorizon")?;
    if m.is_empty() {
        return Err("mpc: ControlHorizon cannot be empty".into());
    }
    if m.rows() != 1 && m.cols() != 1 {
        return Err("mpc: ControlHorizon must be a positive integer scalar or vector".into());
    }
    let mut vals = Vec::with_capacity(m.len());
    for &x in m.data() {
        if !x.is_finite() || x < 1. || x.fract() != 0. || x > p as f64 {
            return Err(format!(
                "mpc: ControlHorizon entries must be positive integers <= PredictionHorizon ({p})"
            ));
        }
        vals.push(x as usize);
    }
    if vals.len() == 1 {
        Ok(ControlHorizon::Scalar(vals[0]))
    } else {
        let sum: usize = vals.iter().sum();
        if sum != p {
            return Err(format!(
                "mpc: blocking ControlHorizon vector must sum to PredictionHorizon ({p}), got {sum}"
            ));
        }
        Ok(ControlHorizon::Blocking(vals))
    }
}

fn validate_control_horizon(ch: &ControlHorizon, p: usize) -> Result<()> {
    match ch {
        ControlHorizon::Scalar(m) if *m >= 1 && *m <= p => Ok(()),
        ControlHorizon::Scalar(m) => Err(format!(
            "mpc: ControlHorizon ({m}) cannot exceed PredictionHorizon ({p})"
        )),
        ControlHorizon::Blocking(v) => {
            let sum: usize = v.iter().sum();
            if sum == p {
                Ok(())
            } else {
                Err(format!(
                    "mpc: blocking ControlHorizon vector sums to {sum}, which does not match PredictionHorizon ({p})"
                ))
            }
        }
    }
}

fn parse_weight_matrix(a: &Array, p: usize, n: usize, name: &str) -> Result<Matrix> {
    let m = real_matrix(a, name)?;
    if m.data().iter().any(|&x| !x.is_finite() || x < 0.) {
        return Err(format!(
            "mpc: {name} must contain finite non-negative values"
        ));
    }
    if m.len() == 1 {
        return Matrix::filled(1, n, m.data()[0]);
    }
    if m.rows() == 1 && m.cols() == n {
        return Ok(m.clone());
    }
    if m.rows() == p && m.cols() == n {
        return Ok(m.clone());
    }
    Err(format!(
        "mpc: {name} must be a scalar, a 1-by-{n} row vector, or a {p}-by-{n} matrix"
    ))
}

fn parse_bound_vec(
    a: &Array,
    p: usize,
    allow_neg_inf: bool,
    allow_pos_inf: bool,
    finite_nonneg: bool,
    name: &str,
) -> Result<Matrix> {
    let m = real_matrix(a, name)?;
    if m.is_empty() {
        return Err(format!("mpc: {name} cannot be empty"));
    }
    if m.rows() != 1 && m.cols() != 1 {
        return Err(format!("mpc: {name} must be a scalar or vector"));
    }
    if m.len() > p {
        return Err(format!(
            "mpc: {name} length ({}) cannot exceed PredictionHorizon ({p})",
            m.len()
        ));
    }
    for &x in m.data() {
        if x.is_nan() {
            return Err(format!("mpc: {name} cannot contain NaN"));
        }
        if finite_nonneg && (!x.is_finite() || x < 0.) {
            return Err(format!("mpc: {name} must be finite and non-negative"));
        }
        if !finite_nonneg {
            if x == f64::NEG_INFINITY && !allow_neg_inf {
                return Err(format!("mpc: {name} cannot be -Inf"));
            }
            if x == f64::INFINITY && !allow_pos_inf {
                return Err(format!("mpc: {name} cannot be +Inf"));
            }
        }
    }
    if m.len() == 1 {
        Ok(Matrix::scalar(m.data()[0]))
    } else {
        Matrix::from_column_vec(m.len(), 1, m.data().to_vec())
    }
}

fn check_min_le_max(min: &Matrix, max: &Matrix, label: &str) -> Result<()> {
    let n = min.len().max(max.len());
    for i in 0..n {
        let lo = if i < min.len() {
            min.data()[i]
        } else {
            *min.data().last().unwrap()
        };
        let hi = if i < max.len() {
            max.data()[i]
        } else {
            *max.data().last().unwrap()
        };
        if lo > hi {
            return Err(format!("mpc: {label} Min must be <= Max"));
        }
    }
    Ok(())
}

fn parse_nominal_vec(a: &Array, len: usize, name: &str) -> Result<Matrix> {
    let m = real_matrix(a, name)?;
    if m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("mpc: {name} must contain finite values"));
    }
    if len == 0 && m.is_empty() {
        return Matrix::filled(0, 1, 0.);
    }
    if (m.rows() != 1 && m.cols() != 1) || m.len() != len {
        return Err(format!("mpc: {name} must be a vector of length {len}"));
    }
    Matrix::from_column_vec(len, 1, m.data().to_vec())
}

fn extract_signal_names(
    a: &Array,
    count: usize,
    cancel: &std::sync::atomic::AtomicBool,
) -> Vec<String> {
    match a {
        Array::Character(c) if count == 1 => vec![c.to_utf8(cancel).unwrap_or_default()],
        Array::Cell(cell) => (0..count)
            .map(|i| {
                cell.get_linear(i)
                    .ok()
                    .and_then(|item| match &**item {
                        Array::Character(c) => c.to_utf8(cancel).ok(),
                        _ => None,
                    })
                    .unwrap_or_default()
            })
            .collect(),
        _ => vec![String::new(); count],
    }
}

fn parse_group_indices(
    group_arr: &Array,
    total: usize,
    aliases: &[(&str, usize)],
    kind: &str,
) -> Result<Vec<usize>> {
    let mut assigned = vec![None; total];
    let Array::Struct(s) = group_arr else {
        return Err(format!("mpc: plant {kind} must be a structure"));
    };
    if s.is_empty() {
        return Ok(assigned.into_iter().map(|x| x.unwrap_or(0)).collect());
    }
    if s.len() != 1 {
        return Err(format!("mpc: plant {kind} must be a scalar structure"));
    }
    for (fi, fname) in s.field_names().iter().enumerate() {
        let Some(&(_, cat)) = aliases.iter().find(|(k, _)| *k == fname.as_str()) else {
            continue;
        };
        let val = s.get(fi, 0)?;
        let m = real_matrix(val.as_ref(), fname)?;
        for &raw in m.data() {
            if !raw.is_finite() || raw < 1. || raw.fract() != 0. || raw as usize > total {
                return Err(format!(
                    "mpc: {kind}.{fname} index {raw} is out of range 1..{total}"
                ));
            }
            let idx = raw as usize - 1;
            if assigned[idx].is_some() {
                return Err(format!(
                    "mpc: channel {} appears in multiple {kind} groups",
                    idx + 1
                ));
            }
            assigned[idx] = Some(cat);
        }
    }
    Ok(assigned.into_iter().map(|x| x.unwrap_or(0)).collect())
}

fn convert_plant(
    e: &mut Engine,
    plant_arr: &Array,
    ts_arg: Option<f64>,
    out: &mut Execution,
) -> Result<(Lti, f64, Matrix, Matrix, Matrix, Matrix)> {
    let lti = control_lti::decode(e, plant_arr)?
        .ok_or("mpc: first argument must be an LTI model (tf, zpk, or ss)")?;
    lti.require_delay_free("mpc plant")?;
    let ts = if lti.ts == 0. {
        ts_arg.ok_or("mpc: Ts must be specified for a continuous-time plant")?
    } else if let Some(t) = ts_arg {
        if (t - lti.ts).abs() > 1e-12 * lti.ts.max(1.) {
            return Err(format!(
                "mpc: Ts ({t}) does not match discrete plant Ts ({}); d2d resampling is not supported",
                lti.ts
            ));
        }
        lti.ts
    } else {
        lti.ts
    };
    if !ts.is_finite() || ts <= 0. {
        return Err("mpc: Ts must be a positive finite scalar".into());
    }
    let disc = if lti.ts == 0. {
        e.ctl_sample(&lti, ts, "zoh", false, out)?
    } else {
        lti.clone()
    };
    let (ad, bd, cd, dd) = e.ctl_ss(&disc, out)?;
    Ok((lti, ts, ad, bd, cd, dd))
}

fn reject_mv_direct_feedthrough(dd: &Matrix, mv_idx: &[usize]) -> Result<()> {
    if mv_idx
        .iter()
        .any(|&input| (0..dd.rows()).any(|output| dd.get(output, input) != 0.))
    {
        return Err(
            "mpc: direct feedthrough from manipulated variables to outputs is not supported".into(),
        );
    }
    Ok(())
}

pub(super) struct SignalPartition {
    pub(super) mv_idx: Vec<usize>,
    pub(super) md_idx: Vec<usize>,
    pub(super) ud_idx: Vec<usize>,
    pub(super) mo_idx: Vec<usize>,
    pub(super) uo_idx: Vec<usize>,
    in_names: Vec<String>,
    out_names: Vec<String>,
}

pub(super) fn init_signals(
    e: &Engine,
    plant_arr: &Array,
    nu: usize,
    ny: usize,
) -> Result<SignalPartition> {
    let Array::Struct(s) = plant_arr else {
        return Err("mpc: plant must be an LTI model".into());
    };
    let in_names = s
        .field_index("InputName")
        .and_then(|i| s.get(i, 0).ok())
        .map(|a| extract_signal_names(a.as_ref(), nu, &e.cancel))
        .unwrap_or_else(|| vec![String::new(); nu]);
    let out_names = s
        .field_index("OutputName")
        .and_then(|i| s.get(i, 0).ok())
        .map(|a| extract_signal_names(a.as_ref(), ny, &e.cancel))
        .unwrap_or_else(|| vec![String::new(); ny]);

    let in_cats = if let Some(ig_idx) = s.field_index("InputGroup") {
        let ig = s.get(ig_idx, 0)?;
        parse_group_indices(
            ig.as_ref(),
            nu,
            &[
                ("MV", 0),
                ("Manipulated", 0),
                ("ManipulatedVariables", 0),
                ("MD", 1),
                ("Measured", 1),
                ("MeasuredDisturbances", 1),
                ("UD", 2),
                ("Unmeasured", 2),
                ("UnmeasuredDisturbances", 2),
            ],
            "InputGroup",
        )?
    } else {
        vec![0; nu]
    };

    let out_cats = if let Some(og_idx) = s.field_index("OutputGroup") {
        let og = s.get(og_idx, 0)?;
        parse_group_indices(
            og.as_ref(),
            ny,
            &[
                ("MO", 0),
                ("Measured", 0),
                ("MeasuredOutputs", 0),
                ("UO", 1),
                ("Unmeasured", 1),
                ("UnmeasuredOutputs", 1),
            ],
            "OutputGroup",
        )?
    } else {
        vec![0; ny]
    };

    let mut mv_idx = Vec::new();
    let mut md_idx = Vec::new();
    let mut ud_idx = Vec::new();
    for (i, cat) in in_cats.into_iter().enumerate() {
        match cat {
            0 => mv_idx.push(i),
            1 => md_idx.push(i),
            _ => ud_idx.push(i),
        }
    }
    if mv_idx.is_empty() {
        return Err("mpc: plant must have at least one manipulated variable (MV)".into());
    }
    let mut mo_idx = Vec::new();
    let mut uo_idx = Vec::new();
    for (i, cat) in out_cats.into_iter().enumerate() {
        match cat {
            0 => mo_idx.push(i),
            _ => uo_idx.push(i),
        }
    }
    if mo_idx.is_empty() {
        return Err("mpc: plant must have at least one measured output (MO)".into());
    }
    Ok(SignalPartition {
        mv_idx,
        md_idx,
        ud_idx,
        mo_idx,
        uo_idx,
        in_names,
        out_names,
    })
}

fn default_mv(name: String) -> MvSpec {
    MvSpec {
        min: Matrix::scalar(f64::NEG_INFINITY),
        max: Matrix::scalar(f64::INFINITY),
        rate_min: Matrix::scalar(f64::NEG_INFINITY),
        rate_max: Matrix::scalar(f64::INFINITY),
        min_ecr: Matrix::scalar(0.),
        max_ecr: Matrix::scalar(0.),
        rate_min_ecr: Matrix::scalar(0.),
        rate_max_ecr: Matrix::scalar(0.),
        target: MvTarget::Nominal,
        name,
        units: String::new(),
        scale_factor: 1.,
    }
}

fn default_ov(name: String) -> OvSpec {
    OvSpec {
        min: Matrix::scalar(f64::NEG_INFINITY),
        max: Matrix::scalar(f64::INFINITY),
        min_ecr: Matrix::scalar(1.),
        max_ecr: Matrix::scalar(1.),
        name,
        units: String::new(),
        scale_factor: 1.,
    }
}

pub(super) fn construct(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > 1 {
        return Err("mpc: too many output arguments".into());
    }
    if args.is_empty() {
        return Err("mpc: expected at least one input argument (plant)".into());
    }
    if args.len() > 8 {
        return Err(
            "mpc: expected at most 8 positional arguments (plant,Ts,p,m,W,MV,OV,DV)".into(),
        );
    }

    let plant_arr = args[0].clone().array(&e.cancel)?;
    let ts_arg = if args.len() > 1 && !is_empty_arg(&args[1]) {
        let a = args[1].clone().array(&e.cancel)?;
        Some(positive_finite_scalar(&a, "Ts")?)
    } else {
        None
    };

    let (_, ts, ad, bd, cd, dd) = convert_plant(e, &plant_arr, ts_arg, out)?;
    let nx = ad.rows();
    let nu = bd.cols();
    let ny = cd.rows();
    let SignalPartition {
        mv_idx,
        md_idx,
        ud_idx,
        mo_idx,
        uo_idx,
        in_names,
        out_names,
    } = init_signals(e, &plant_arr, nu, ny)?;
    reject_mv_direct_feedthrough(&dd, &mv_idx)?;

    let p = if args.len() > 2 && !is_empty_arg(&args[2]) {
        let a = args[2].clone().array(&e.cancel)?;
        positive_int_scalar(&a, "PredictionHorizon", MAX_HORIZON)?
    } else {
        10
    };

    let control_horizon = if args.len() > 3 && !is_empty_arg(&args[3]) {
        let a = args[3].clone().array(&e.cancel)?;
        parse_control_horizon(&a, p)?
    } else {
        ControlHorizon::Scalar(2.min(p))
    };

    let n_mv = mv_idx.len();
    let n_ov = mo_idx.len() + uo_idx.len();
    let weights = MpcWeights {
        mv: Matrix::filled(1, n_mv, 0.)?,
        mv_rate: Matrix::filled(1, n_mv, 0.1)?,
        ov: Matrix::filled(1, n_ov, 1.)?,
        ecr: 1e5,
    };

    let mv: Vec<MvSpec> = mv_idx
        .iter()
        .map(|&i| default_mv(in_names[i].clone()))
        .collect();
    // Public OV properties use physical plant output order.
    let ov: Vec<OvSpec> = out_names.iter().cloned().map(default_ov).collect();
    let dv: Vec<DvSpec> = md_idx
        .iter()
        .chain(ud_idx.iter())
        .map(|&i| DvSpec {
            name: in_names[i].clone(),
            units: String::new(),
            scale_factor: 1.,
        })
        .collect();

    let notes = Array::Cell(CellArray::from_shared(0, 0, Vec::new(), &e.cancel)?);
    let mut obj = MpcObject {
        plant_array: plant_arr,
        ad,
        bd,
        cd,
        dd,
        mv_idx,
        md_idx,
        ud_idx,
        mo_idx,
        uo_idx,
        ts,
        prediction_horizon: p,
        control_horizon,
        weights,
        mv,
        ov,
        dv,
        disturbance: empty_matrix(),
        output_disturbance: empty_matrix(),
        estimator: super::estimator::EstimatorMode::Default,
        noise: empty_matrix(),
        nominal: MpcNominal {
            x: Matrix::filled(nx, 1, 0.)?,
            u: Matrix::filled(nu, 1, 0.)?,
            y: Matrix::filled(ny, 1, 0.)?,
            dx: Matrix::filled(nx, 1, 0.)?,
        },
        optimizer: MpcOptimizer {
            max_iterations: 120,
            constraint_tolerance: 1e-6,
            algorithm: "active-set".into(),
        },
        notes,
        user_data: empty_matrix(),
        history: "Created".into(),
    };

    if args.len() > 4 && !is_empty_arg(&args[4]) {
        let w = args[4].clone().array(&e.cancel)?;
        assign_weights(&mut obj, &w, e)?;
    }
    if args.len() > 5 && !is_empty_arg(&args[5]) {
        let a = args[5].clone().array(&e.cancel)?;
        assign_mv_array(&mut obj, &a, e)?;
    }
    if args.len() > 6 && !is_empty_arg(&args[6]) {
        let a = args[6].clone().array(&e.cancel)?;
        assign_ov_array(&mut obj, &a, e)?;
    }
    if args.len() > 7 && !is_empty_arg(&args[7]) {
        let a = args[7].clone().array(&e.cancel)?;
        assign_dv_array(&mut obj, &a, e)?;
    }

    Ok(vec![super::super::objects::mpc_new(e, obj)?])
}

fn weights_struct(e: &Engine, w: &MpcWeights) -> Result<Array> {
    struct_arr(
        e,
        vec![
            ("ManipulatedVariables", Array::Numeric(w.mv.clone())),
            (
                "ManipulatedVariablesRate",
                Array::Numeric(w.mv_rate.clone()),
            ),
            ("OutputVariables", Array::Numeric(w.ov.clone())),
            ("ECR", Array::Numeric(Matrix::scalar(w.ecr))),
            ("MV", Array::Numeric(w.mv.clone())),
            ("MVRate", Array::Numeric(w.mv_rate.clone())),
            ("OV", Array::Numeric(w.ov.clone())),
        ],
    )
}

fn mv_struct_array(e: &Engine, mvs: &[MvSpec]) -> Result<Array> {
    let n = mvs.len();
    let names = vec![
        "Min".into(),
        "Max".into(),
        "RateMin".into(),
        "RateMax".into(),
        "MinECR".into(),
        "MaxECR".into(),
        "RateMinECR".into(),
        "RateMaxECR".into(),
        "Target".into(),
        "Name".into(),
        "Units".into(),
        "ScaleFactor".into(),
    ];
    let mut slots = Vec::with_capacity(n * names.len());
    for mv in mvs {
        let target = match &mv.target {
            MvTarget::Scalar(v) => Array::Numeric(Matrix::scalar(*v)),
            MvTarget::Vector(m) => Array::Numeric(m.clone()),
            MvTarget::Nominal => char_arr(e, "nominal")?,
        };
        slots.push(Arc::new(Array::Numeric(mv.min.clone())));
        slots.push(Arc::new(Array::Numeric(mv.max.clone())));
        slots.push(Arc::new(Array::Numeric(mv.rate_min.clone())));
        slots.push(Arc::new(Array::Numeric(mv.rate_max.clone())));
        slots.push(Arc::new(Array::Numeric(mv.min_ecr.clone())));
        slots.push(Arc::new(Array::Numeric(mv.max_ecr.clone())));
        slots.push(Arc::new(Array::Numeric(mv.rate_min_ecr.clone())));
        slots.push(Arc::new(Array::Numeric(mv.rate_max_ecr.clone())));
        slots.push(Arc::new(target));
        slots.push(Arc::new(char_arr(e, &mv.name)?));
        slots.push(Arc::new(char_arr(e, &mv.units)?));
        slots.push(Arc::new(Array::Numeric(Matrix::scalar(mv.scale_factor))));
    }
    Ok(Array::Struct(StructArray::from_slots(
        1, n, names, slots, &e.cancel,
    )?))
}

fn ov_struct_array(e: &Engine, ovs: &[OvSpec]) -> Result<Array> {
    let n = ovs.len();
    let names = vec![
        "Min".into(),
        "Max".into(),
        "MinECR".into(),
        "MaxECR".into(),
        "Name".into(),
        "Units".into(),
        "ScaleFactor".into(),
    ];
    let mut slots = Vec::with_capacity(n * names.len());
    for ov in ovs {
        slots.push(Arc::new(Array::Numeric(ov.min.clone())));
        slots.push(Arc::new(Array::Numeric(ov.max.clone())));
        slots.push(Arc::new(Array::Numeric(ov.min_ecr.clone())));
        slots.push(Arc::new(Array::Numeric(ov.max_ecr.clone())));
        slots.push(Arc::new(char_arr(e, &ov.name)?));
        slots.push(Arc::new(char_arr(e, &ov.units)?));
        slots.push(Arc::new(Array::Numeric(Matrix::scalar(ov.scale_factor))));
    }
    Ok(Array::Struct(StructArray::from_slots(
        1, n, names, slots, &e.cancel,
    )?))
}

fn dv_struct_array(e: &Engine, dvs: &[DvSpec]) -> Result<Array> {
    let n = dvs.len();
    let names = vec!["Name".into(), "Units".into(), "ScaleFactor".into()];
    let mut slots = Vec::with_capacity(n * names.len());
    for dv in dvs {
        slots.push(Arc::new(char_arr(e, &dv.name)?));
        slots.push(Arc::new(char_arr(e, &dv.units)?));
        slots.push(Arc::new(Array::Numeric(Matrix::scalar(dv.scale_factor))));
    }
    let rows = if n == 0 { 0 } else { 1 };
    Ok(Array::Struct(StructArray::from_slots(
        rows, n, names, slots, &e.cancel,
    )?))
}

fn model_struct(e: &Engine, m: &MpcObject) -> Result<Array> {
    let nominal = struct_arr(
        e,
        vec![
            ("X", Array::Numeric(m.nominal.x.clone())),
            ("U", Array::Numeric(m.nominal.u.clone())),
            ("Y", Array::Numeric(m.nominal.y.clone())),
            ("DX", Array::Numeric(m.nominal.dx.clone())),
        ],
    )?;
    struct_arr(
        e,
        vec![
            ("Plant", m.plant_array.clone()),
            ("Disturbance", m.disturbance.clone()),
            ("Noise", m.noise.clone()),
            ("Nominal", nominal),
        ],
    )
}

fn optimizer_struct(e: &Engine, o: &MpcOptimizer) -> Result<Array> {
    struct_arr(
        e,
        vec![
            ("Algorithm", char_arr(e, &o.algorithm)?),
            (
                "MaxIterations",
                Array::Numeric(Matrix::scalar(o.max_iterations as f64)),
            ),
            (
                "ConstraintTolerance",
                Array::Numeric(Matrix::scalar(o.constraint_tolerance)),
            ),
        ],
    )
}

pub(in crate::engine) fn canonical_prop(name: &str) -> &str {
    match name {
        "p" => "PredictionHorizon",
        "m" => "ControlHorizon",
        "MV" => "ManipulatedVariables",
        "OV" => "OutputVariables",
        "DV" => "DisturbanceVariables",
        other => other,
    }
}

pub(in crate::engine) fn property(e: &Engine, m: &MpcObject, name: &str) -> Result<Value> {
    let prop = canonical_prop(name);
    let arr = match prop {
        "Ts" => Array::Numeric(Matrix::scalar(m.ts)),
        "PredictionHorizon" => Array::Numeric(Matrix::scalar(m.prediction_horizon as f64)),
        "ControlHorizon" => match &m.control_horizon {
            ControlHorizon::Scalar(k) => Array::Numeric(Matrix::scalar(*k as f64)),
            ControlHorizon::Blocking(v) => Array::Numeric(Matrix::from_column_vec(
                1,
                v.len(),
                v.iter().map(|&x| x as f64).collect(),
            )?),
        },
        "Weights" => weights_struct(e, &m.weights)?,
        "ManipulatedVariables" => mv_struct_array(e, &m.mv)?,
        "OutputVariables" => ov_struct_array(e, &m.ov)?,
        "DisturbanceVariables" => dv_struct_array(e, &m.dv)?,
        "Model" => model_struct(e, m)?,
        "Optimizer" => optimizer_struct(e, &m.optimizer)?,
        "Notes" => m.notes.clone(),
        "UserData" => m.user_data.clone(),
        "History" => char_arr(e, &m.history)?,
        _ => {
            return Err(format!(
                "The name '{name}' is not an accessible property for an instance of class 'mpc'."
            ));
        }
    };
    Ok(Value::from(arr))
}

pub(in crate::engine) fn property_names() -> Vec<String> {
    PROPERTY_NAMES.iter().map(|&s| s.to_string()).collect()
}

fn assign_weight_field(
    w: &mut MpcWeights,
    field: &str,
    val: &Array,
    p: usize,
    n_mv: usize,
    n_ov: usize,
) -> Result<()> {
    match field {
        "ManipulatedVariables" | "MV" => {
            w.mv = parse_weight_matrix(val, p, n_mv, "Weights.ManipulatedVariables")?;
        }
        "ManipulatedVariablesRate" | "MVRate" => {
            w.mv_rate = parse_weight_matrix(val, p, n_mv, "Weights.ManipulatedVariablesRate")?;
        }
        "OutputVariables" | "OV" => {
            w.ov = parse_weight_matrix(val, p, n_ov, "Weights.OutputVariables")?;
        }
        "ECR" => {
            let v = real_scalar(val, "Weights.ECR")?;
            if !v.is_finite() || v <= 0. {
                return Err("mpc: Weights.ECR must be positive and finite".into());
            }
            w.ecr = v;
        }
        _ => return Err(format!("mpc: unknown Weights property '{field}'")),
    }
    Ok(())
}

fn assign_weights(obj: &mut MpcObject, val: &Array, _e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: Weights must be a scalar structure".into());
    };
    if s.len() != 1 {
        return Err("mpc: Weights must be a scalar structure".into());
    }
    for fname in s.field_names() {
        if !matches!(
            fname.as_str(),
            "ManipulatedVariables"
                | "MV"
                | "ManipulatedVariablesRate"
                | "MVRate"
                | "OutputVariables"
                | "OV"
                | "ECR"
        ) {
            return Err(format!("mpc: unknown Weights property '{fname}'"));
        }
    }

    let mut next = obj.weights.clone();
    if let Some(value) = resolve_weight_alias(
        s,
        &["ManipulatedVariables", "MV"],
        &obj.weights.mv,
        obj.prediction_horizon,
        obj.n_mv(),
        "Weights.ManipulatedVariables",
    )? {
        next.mv = value;
    }
    if let Some(value) = resolve_weight_alias(
        s,
        &["ManipulatedVariablesRate", "MVRate"],
        &obj.weights.mv_rate,
        obj.prediction_horizon,
        obj.n_mv(),
        "Weights.ManipulatedVariablesRate",
    )? {
        next.mv_rate = value;
    }
    if let Some(value) = resolve_weight_alias(
        s,
        &["OutputVariables", "OV"],
        &obj.weights.ov,
        obj.prediction_horizon,
        obj.n_ov(),
        "Weights.OutputVariables",
    )? {
        next.ov = value;
    }
    if let Some(i) = s.field_index("ECR") {
        assign_weight_field(
            &mut next,
            "ECR",
            s.get(i, 0)?.as_ref(),
            obj.prediction_horizon,
            obj.n_mv(),
            obj.n_ov(),
        )?;
    }
    obj.weights = next;
    Ok(())
}

fn resolve_weight_alias(
    s: &StructArray,
    aliases: &[&str],
    current: &Matrix,
    p: usize,
    n: usize,
    name: &str,
) -> Result<Option<Matrix>> {
    let mut changed = None;
    for &alias in aliases {
        let Some(i) = s.field_index(alias) else {
            continue;
        };
        let candidate = parse_weight_matrix(s.get(i, 0)?.as_ref(), p, n, name)?;
        if matrix_same(&candidate, current) {
            continue;
        }
        if let Some(previous) = &changed {
            if !matrix_same(previous, &candidate) {
                return Err(format!(
                    "mpc: conflicting values for {name} aliases '{}' and '{}'",
                    aliases[0], aliases[1]
                ));
            }
        } else {
            changed = Some(candidate);
        }
    }
    Ok(changed)
}

fn matrix_same(a: &Matrix, b: &Matrix) -> bool {
    a.rows() == b.rows() && a.cols() == b.cols() && a.data() == b.data()
}

fn assign_mv_field(mv: &mut MvSpec, field: &str, val: &Array, p: usize, e: &Engine) -> Result<()> {
    match field {
        "Min" => {
            let next = parse_bound_vec(val, p, true, false, false, "MV.Min")?;
            check_min_le_max(&next, &mv.max, "MV")?;
            mv.min = next;
        }
        "Max" => {
            let next = parse_bound_vec(val, p, false, true, false, "MV.Max")?;
            check_min_le_max(&mv.min, &next, "MV")?;
            mv.max = next;
        }
        "RateMin" => {
            let next = parse_bound_vec(val, p, true, false, false, "MV.RateMin")?;
            check_min_le_max(&next, &mv.rate_max, "MV Rate")?;
            mv.rate_min = next;
        }
        "RateMax" => {
            let next = parse_bound_vec(val, p, false, true, false, "MV.RateMax")?;
            check_min_le_max(&mv.rate_min, &next, "MV Rate")?;
            mv.rate_max = next;
        }
        "MinECR" => mv.min_ecr = parse_bound_vec(val, p, false, false, true, "MV.MinECR")?,
        "MaxECR" => mv.max_ecr = parse_bound_vec(val, p, false, false, true, "MV.MaxECR")?,
        "RateMinECR" => {
            mv.rate_min_ecr = parse_bound_vec(val, p, false, false, true, "MV.RateMinECR")?
        }
        "RateMaxECR" => {
            mv.rate_max_ecr = parse_bound_vec(val, p, false, false, true, "MV.RateMaxECR")?
        }
        "Target" => {
            if let Some(t) = text_opt(e, val) {
                if t.eq_ignore_ascii_case("nominal") {
                    mv.target = MvTarget::Nominal;
                } else {
                    return Err("mpc: MV.Target must be numeric or 'nominal'".into());
                }
            } else {
                let m = parse_bound_vec(val, p, false, false, false, "MV.Target")?;
                if m.data().iter().any(|x| !x.is_finite()) {
                    return Err("mpc: MV.Target must be finite".into());
                }
                mv.target = if m.len() == 1 {
                    MvTarget::Scalar(m.data()[0])
                } else {
                    MvTarget::Vector(m)
                };
            }
        }
        "Name" => mv.name = text_req(e, val, "MV.Name")?,
        "Units" => mv.units = text_req(e, val, "MV.Units")?,
        "ScaleFactor" => mv.scale_factor = positive_finite_scalar(val, "MV.ScaleFactor")?,
        _ => return Err(format!("mpc: unknown MV property '{field}'")),
    }
    Ok(())
}

fn assign_ov_field(ov: &mut OvSpec, field: &str, val: &Array, p: usize, e: &Engine) -> Result<()> {
    match field {
        "Min" => {
            let next = parse_bound_vec(val, p, true, false, false, "OV.Min")?;
            check_min_le_max(&next, &ov.max, "OV")?;
            ov.min = next;
        }
        "Max" => {
            let next = parse_bound_vec(val, p, false, true, false, "OV.Max")?;
            check_min_le_max(&ov.min, &next, "OV")?;
            ov.max = next;
        }
        "MinECR" => ov.min_ecr = parse_bound_vec(val, p, false, false, true, "OV.MinECR")?,
        "MaxECR" => ov.max_ecr = parse_bound_vec(val, p, false, false, true, "OV.MaxECR")?,
        "Name" => ov.name = text_req(e, val, "OV.Name")?,
        "Units" => ov.units = text_req(e, val, "OV.Units")?,
        "ScaleFactor" => ov.scale_factor = positive_finite_scalar(val, "OV.ScaleFactor")?,
        _ => return Err(format!("mpc: unknown OV property '{field}'")),
    }
    Ok(())
}

fn assign_mv_field_without_bounds_check(
    mv: &mut MvSpec,
    field: &str,
    val: &Array,
    p: usize,
    e: &Engine,
) -> Result<()> {
    match field {
        "Min" => mv.min = parse_bound_vec(val, p, true, false, false, "MV.Min")?,
        "Max" => mv.max = parse_bound_vec(val, p, false, true, false, "MV.Max")?,
        "RateMin" => mv.rate_min = parse_bound_vec(val, p, true, false, false, "MV.RateMin")?,
        "RateMax" => mv.rate_max = parse_bound_vec(val, p, false, true, false, "MV.RateMax")?,
        _ => assign_mv_field(mv, field, val, p, e)?,
    }
    Ok(())
}

fn validate_mv_bounds(mv: &MvSpec) -> Result<()> {
    check_min_le_max(&mv.min, &mv.max, "MV")?;
    check_min_le_max(&mv.rate_min, &mv.rate_max, "MV Rate")
}

fn assign_ov_field_without_bounds_check(
    ov: &mut OvSpec,
    field: &str,
    val: &Array,
    p: usize,
    e: &Engine,
) -> Result<()> {
    match field {
        "Min" => ov.min = parse_bound_vec(val, p, true, false, false, "OV.Min")?,
        "Max" => ov.max = parse_bound_vec(val, p, false, true, false, "OV.Max")?,
        _ => assign_ov_field(ov, field, val, p, e)?,
    }
    Ok(())
}

fn validate_ov_bounds(ov: &OvSpec) -> Result<()> {
    check_min_le_max(&ov.min, &ov.max, "OV")
}

fn assign_dv_field(dv: &mut DvSpec, field: &str, val: &Array, e: &Engine) -> Result<()> {
    match field {
        "Name" => dv.name = text_req(e, val, "DV.Name")?,
        "Units" => dv.units = text_req(e, val, "DV.Units")?,
        "ScaleFactor" => dv.scale_factor = positive_finite_scalar(val, "DV.ScaleFactor")?,
        _ => return Err(format!("mpc: unknown DV property '{field}'")),
    }
    Ok(())
}

fn assign_mv_elem(mv: &mut MvSpec, val: &Array, p: usize, e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: MV element must be a scalar structure".into());
    };
    if s.len() != 1 {
        return Err("mpc: MV element must be a scalar structure".into());
    }
    for (i, fname) in s.field_names().iter().enumerate() {
        assign_mv_field_without_bounds_check(mv, fname, s.get(i, 0)?.as_ref(), p, e)?;
    }
    validate_mv_bounds(mv)?;
    Ok(())
}

fn assign_ov_elem(ov: &mut OvSpec, val: &Array, p: usize, e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: OV element must be a scalar structure".into());
    };
    if s.len() != 1 {
        return Err("mpc: OV element must be a scalar structure".into());
    }
    for (i, fname) in s.field_names().iter().enumerate() {
        assign_ov_field_without_bounds_check(ov, fname, s.get(i, 0)?.as_ref(), p, e)?;
    }
    validate_ov_bounds(ov)?;
    Ok(())
}

fn assign_dv_elem(dv: &mut DvSpec, val: &Array, e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: DV element must be a scalar structure".into());
    };
    if s.len() != 1 {
        return Err("mpc: DV element must be a scalar structure".into());
    }
    for (i, fname) in s.field_names().iter().enumerate() {
        assign_dv_field(dv, fname, s.get(i, 0)?.as_ref(), e)?;
    }
    Ok(())
}

fn assign_mv_array(obj: &mut MpcObject, val: &Array, e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: ManipulatedVariables must be a structure array".into());
    };
    if s.len() != obj.mv.len() {
        return Err(format!(
            "mpc: ManipulatedVariables must have {} element(s), got {}",
            obj.mv.len(),
            s.len()
        ));
    }
    let mut next = obj.mv.clone();
    for (idx, mv) in next.iter_mut().enumerate() {
        for (fi, fname) in s.field_names().iter().enumerate() {
            assign_mv_field_without_bounds_check(
                mv,
                fname,
                s.get(fi, idx)?.as_ref(),
                obj.prediction_horizon,
                e,
            )?;
        }
        validate_mv_bounds(mv)?;
    }
    obj.mv = next;
    Ok(())
}

fn assign_ov_array(obj: &mut MpcObject, val: &Array, e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: OutputVariables must be a structure array".into());
    };
    if s.len() != obj.ov.len() {
        return Err(format!(
            "mpc: OutputVariables must have {} element(s), got {}",
            obj.ov.len(),
            s.len()
        ));
    }
    let mut next = obj.ov.clone();
    for (idx, ov) in next.iter_mut().enumerate() {
        for (fi, fname) in s.field_names().iter().enumerate() {
            assign_ov_field_without_bounds_check(
                ov,
                fname,
                s.get(fi, idx)?.as_ref(),
                obj.prediction_horizon,
                e,
            )?;
        }
        validate_ov_bounds(ov)?;
    }
    obj.ov = next;
    Ok(())
}

fn assign_dv_array(obj: &mut MpcObject, val: &Array, e: &Engine) -> Result<()> {
    let Array::Struct(s) = val else {
        return Err("mpc: DisturbanceVariables must be a structure array".into());
    };
    if s.len() != obj.dv.len() {
        return Err(format!(
            "mpc: DisturbanceVariables must have {} element(s), got {}",
            obj.dv.len(),
            s.len()
        ));
    }
    let mut next = obj.dv.clone();
    for (idx, dv) in next.iter_mut().enumerate() {
        for (fi, fname) in s.field_names().iter().enumerate() {
            assign_dv_field(dv, fname, s.get(fi, idx)?.as_ref(), e)?;
        }
    }
    obj.dv = next;
    Ok(())
}

fn assign_nominal_field(
    nom: &mut MpcNominal,
    field: &str,
    val: &Array,
    nx: usize,
    nu: usize,
    ny: usize,
) -> Result<()> {
    match field {
        "X" => nom.x = parse_nominal_vec(val, nx, "Model.Nominal.X")?,
        "U" => nom.u = parse_nominal_vec(val, nu, "Model.Nominal.U")?,
        "Y" => nom.y = parse_nominal_vec(val, ny, "Model.Nominal.Y")?,
        "DX" => nom.dx = parse_nominal_vec(val, nx, "Model.Nominal.DX")?,
        _ => return Err(format!("mpc: unknown Model.Nominal property '{field}'")),
    }
    Ok(())
}

fn assign_optimizer_field(
    opt: &mut MpcOptimizer,
    field: &str,
    val: &Array,
    e: &Engine,
) -> Result<()> {
    match field {
        "MaxIterations" => {
            opt.max_iterations =
                positive_int_scalar(val, "Optimizer.MaxIterations", MAX_ITERATIONS)?;
        }
        "ConstraintTolerance" => {
            opt.constraint_tolerance =
                positive_finite_scalar(val, "Optimizer.ConstraintTolerance")?;
        }
        "Algorithm" => {
            let alg = text_req(e, val, "Optimizer.Algorithm")?;
            if alg != "active-set" {
                return Err(format!(
                    "mpc: unsupported Optimizer.Algorithm '{alg}'; only 'active-set' is supported"
                ));
            }
            opt.algorithm = alg;
        }
        _ => return Err(format!("mpc: unknown Optimizer property '{field}'")),
    }
    Ok(())
}

fn parse_struct_index(
    e: &mut Engine,
    step: &PathStep,
    len: usize,
    kind: &str,
    out: &mut Execution,
) -> Result<usize> {
    let PathStep::Paren(args) = step else {
        return Err(format!("mpc: invalid indexing on {kind}"));
    };
    if args.len() != 1 {
        return Err(format!(
            "mpc: {kind} indexing requires a single 1-based index"
        ));
    }
    let v = e.eval(&args[0], None, out)?.array(&e.cancel)?;
    let idx = positive_int_scalar(&v, &format!("{kind} index"), len)?;
    Ok(idx - 1)
}

fn step_field(e: &mut Engine, step: &PathStep, out: &mut Execution) -> Result<String> {
    match step {
        PathStep::Field(name) => Ok(name.clone()),
        PathStep::Dynamic(expr) => {
            let v = e.eval(expr, None, out)?.array(&e.cancel)?;
            text_req(e, &v, "dynamic property name")
        }
        _ => Err("mpc: invalid property path".into()),
    }
}

pub(in crate::engine) fn assign(
    e: &mut Engine,
    current: &MpcObject,
    steps: &[PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<MpcObject> {
    let mut obj = current.clone();
    let Some((first, rest)) = steps.split_first() else {
        return Err("mpc: indexed assignment into mpc object is not supported".into());
    };
    let top_raw = step_field(e, first, out)?;
    let top = canonical_prop(&top_raw);

    match (top, rest) {
        ("Ts", []) => {
            let ts = positive_finite_scalar(&value, "Ts")?;
            let (_, new_ts, ad, bd, cd, dd) = convert_plant(e, &obj.plant_array, Some(ts), out)?;
            obj.ts = new_ts;
            obj.ad = ad;
            obj.bd = bd;
            obj.cd = cd;
            obj.dd = dd;
        }
        ("PredictionHorizon", []) => {
            let p = positive_int_scalar(&value, "PredictionHorizon", MAX_HORIZON)?;
            validate_control_horizon(&obj.control_horizon, p)?;
            if obj.weights.mv.rows() > 1 && obj.weights.mv.rows() != p {
                return Err(format!(
                    "mpc: PredictionHorizon ({p}) mismatches time-varying Weights.MV rows ({})",
                    obj.weights.mv.rows()
                ));
            }
            if obj.weights.mv_rate.rows() > 1 && obj.weights.mv_rate.rows() != p {
                return Err(format!(
                    "mpc: PredictionHorizon ({p}) mismatches time-varying Weights.MVRate rows ({})",
                    obj.weights.mv_rate.rows()
                ));
            }
            if obj.weights.ov.rows() > 1 && obj.weights.ov.rows() != p {
                return Err(format!(
                    "mpc: PredictionHorizon ({p}) mismatches time-varying Weights.OV rows ({})",
                    obj.weights.ov.rows()
                ));
            }
            obj.prediction_horizon = p;
        }
        ("ControlHorizon", []) => {
            obj.control_horizon = parse_control_horizon(&value, obj.prediction_horizon)?;
        }
        ("Weights", []) => assign_weights(&mut obj, &value, e)?,
        ("Weights", [sub]) => {
            let fname = step_field(e, sub, out)?;
            let (n_mv, n_ov) = (obj.n_mv(), obj.n_ov());
            assign_weight_field(
                &mut obj.weights,
                &fname,
                &value,
                obj.prediction_horizon,
                n_mv,
                n_ov,
            )?;
        }
        ("ManipulatedVariables", []) => assign_mv_array(&mut obj, &value, e)?,
        ("ManipulatedVariables", [idx_step]) => {
            if matches!(idx_step, PathStep::Paren(_)) {
                let idx = parse_struct_index(e, idx_step, obj.mv.len(), "MV", out)?;
                assign_mv_elem(&mut obj.mv[idx], &value, obj.prediction_horizon, e)?;
            } else if obj.mv.len() == 1 {
                let fname = step_field(e, idx_step, out)?;
                assign_mv_field(&mut obj.mv[0], &fname, &value, obj.prediction_horizon, e)?;
            } else {
                return Err(
                    "mpc: index MV(i) before accessing a field when there are multiple MVs".into(),
                );
            }
        }
        ("ManipulatedVariables", [idx_step, field_step]) => {
            let idx = parse_struct_index(e, idx_step, obj.mv.len(), "MV", out)?;
            let fname = step_field(e, field_step, out)?;
            assign_mv_field(&mut obj.mv[idx], &fname, &value, obj.prediction_horizon, e)?;
        }
        ("OutputVariables", []) => assign_ov_array(&mut obj, &value, e)?,
        ("OutputVariables", [idx_step]) => {
            if matches!(idx_step, PathStep::Paren(_)) {
                let idx = parse_struct_index(e, idx_step, obj.ov.len(), "OV", out)?;
                assign_ov_elem(&mut obj.ov[idx], &value, obj.prediction_horizon, e)?;
            } else if obj.ov.len() == 1 {
                let fname = step_field(e, idx_step, out)?;
                assign_ov_field(&mut obj.ov[0], &fname, &value, obj.prediction_horizon, e)?;
            } else {
                return Err(
                    "mpc: index OV(i) before accessing a field when there are multiple OVs".into(),
                );
            }
        }
        ("OutputVariables", [idx_step, field_step]) => {
            let idx = parse_struct_index(e, idx_step, obj.ov.len(), "OV", out)?;
            let fname = step_field(e, field_step, out)?;
            assign_ov_field(&mut obj.ov[idx], &fname, &value, obj.prediction_horizon, e)?;
        }
        ("DisturbanceVariables", []) => assign_dv_array(&mut obj, &value, e)?,
        ("DisturbanceVariables", [idx_step]) => {
            if obj.dv.is_empty() {
                return Err("mpc: controller has no disturbance variables (DV)".into());
            }
            if matches!(idx_step, PathStep::Paren(_)) {
                let idx = parse_struct_index(e, idx_step, obj.dv.len(), "DV", out)?;
                assign_dv_elem(&mut obj.dv[idx], &value, e)?;
            } else if obj.dv.len() == 1 {
                let fname = step_field(e, idx_step, out)?;
                assign_dv_field(&mut obj.dv[0], &fname, &value, e)?;
            } else {
                return Err(
                    "mpc: index DV(i) before accessing a field when there are multiple DVs".into(),
                );
            }
        }
        ("DisturbanceVariables", [idx_step, field_step]) => {
            if obj.dv.is_empty() {
                return Err("mpc: controller has no disturbance variables (DV)".into());
            }
            let idx = parse_struct_index(e, idx_step, obj.dv.len(), "DV", out)?;
            let fname = step_field(e, field_step, out)?;
            assign_dv_field(&mut obj.dv[idx], &fname, &value, e)?;
        }
        ("Model", []) => {
            let Array::Struct(s) = &value else {
                return Err("mpc: Model must be a scalar structure".into());
            };
            if s.len() != 1 {
                return Err("mpc: Model must be a scalar structure".into());
            }
            for (i, fname) in s.field_names().iter().enumerate() {
                let sub = s.get(i, 0)?.as_ref().clone();
                obj = assign(
                    e,
                    &obj,
                    &[
                        PathStep::Field("Model".into()),
                        PathStep::Field(fname.clone()),
                    ],
                    sub,
                    out,
                )?;
            }
        }
        ("Model", [sub]) => {
            let fname = step_field(e, sub, out)?;
            match fname.as_str() {
                "Plant" => {
                    let (_, new_ts, ad, bd, cd, dd) = convert_plant(e, &value, Some(obj.ts), out)?;
                    if bd.cols() != obj.nu() || cd.rows() != obj.ny() {
                        return Err(format!(
                            "mpc: Model.Plant must have {} input(s) and {} output(s)",
                            obj.nu(),
                            obj.ny()
                        ));
                    }
                    reject_mv_direct_feedthrough(&dd, &obj.mv_idx)?;
                    let old_nx = obj.nx();
                    obj.plant_array = value;
                    obj.ts = new_ts;
                    obj.ad = ad;
                    obj.bd = bd;
                    obj.cd = cd;
                    obj.dd = dd;
                    if obj.nx() != old_nx {
                        obj.nominal.x = Matrix::filled(obj.nx(), 1, 0.)?;
                        obj.nominal.dx = Matrix::filled(obj.nx(), 1, 0.)?;
                    }
                }
                "Disturbance" => {
                    super::prediction::decode_model(
                        e,
                        &value,
                        obj.ud_idx.len(),
                        obj.ts,
                        "Model.Disturbance",
                        out,
                    )?;
                    obj.disturbance = value;
                }
                "Noise" => {
                    super::prediction::decode_model(
                        e,
                        &value,
                        obj.mo_idx.len(),
                        obj.ts,
                        "Model.Noise",
                        out,
                    )?;
                    obj.noise = value;
                }
                "Nominal" => {
                    let Array::Struct(s) = &value else {
                        return Err("mpc: Model.Nominal must be a scalar structure".into());
                    };
                    if s.len() != 1 {
                        return Err("mpc: Model.Nominal must be a scalar structure".into());
                    }
                    let (nx, nu, ny) = (obj.nx(), obj.nu(), obj.ny());
                    for (i, nf) in s.field_names().iter().enumerate() {
                        assign_nominal_field(
                            &mut obj.nominal,
                            nf,
                            s.get(i, 0)?.as_ref(),
                            nx,
                            nu,
                            ny,
                        )?;
                    }
                }
                _ => return Err(format!("mpc: unknown Model property '{fname}'")),
            }
        }
        ("Model", [sub, leaf]) => {
            let fname = step_field(e, sub, out)?;
            if fname != "Nominal" {
                return Err(format!("mpc: cannot index into Model.{fname}"));
            }
            let nfield = step_field(e, leaf, out)?;
            let (nx, nu, ny) = (obj.nx(), obj.nu(), obj.ny());
            assign_nominal_field(&mut obj.nominal, &nfield, &value, nx, nu, ny)?;
        }
        ("Optimizer", []) => {
            let Array::Struct(s) = &value else {
                return Err("mpc: Optimizer must be a scalar structure".into());
            };
            if s.len() != 1 {
                return Err("mpc: Optimizer must be a scalar structure".into());
            }
            for (i, fname) in s.field_names().iter().enumerate() {
                assign_optimizer_field(&mut obj.optimizer, fname, s.get(i, 0)?.as_ref(), e)?;
            }
        }
        ("Optimizer", [sub]) => {
            let fname = step_field(e, sub, out)?;
            assign_optimizer_field(&mut obj.optimizer, &fname, &value, e)?;
        }
        ("Notes", []) => match &value {
            Array::Cell(_) => obj.notes = value,
            Array::Character(_) | Array::Str(_) => {
                let s = text_req(e, &value, "Notes")?;
                obj.notes = Array::Cell(CellArray::from_shared(
                    1,
                    1,
                    vec![Arc::new(char_arr(e, &s)?)],
                    &e.cancel,
                )?);
            }
            _ => {
                return Err(
                    "mpc: Notes must be a cell array of strings or character vector".into(),
                );
            }
        },
        ("UserData", []) => obj.user_data = value,
        ("History", []) => return Err("mpc: property 'History' is read-only".into()),
        (p, _) if PROPERTY_NAMES.contains(&p) => {
            return Err(format!("mpc: invalid nested assignment on '{top_raw}'"));
        }
        _ => {
            return Err(format!(
                "The name '{top_raw}' is not an accessible property for an instance of class 'mpc'."
            ));
        }
    }
    // Only actual changes to estimator inputs reset supplied/custom estimation.
    // Bounds and display metadata do not enter the prediction/estimation model.
    let reset_estimator = match top {
        "Ts" => obj.ts != current.ts,
        "Weights" => {
            !mat_eq(&obj.weights.mv, &current.weights.mv)
                || !mat_eq(&obj.weights.mv_rate, &current.weights.mv_rate)
                || !mat_eq(&obj.weights.ov, &current.weights.ov)
                || obj.weights.ecr != current.weights.ecr
        }
        "ManipulatedVariables" => obj
            .mv
            .iter()
            .zip(&current.mv)
            .any(|(a, b)| a.scale_factor != b.scale_factor),
        "OutputVariables" => obj
            .ov
            .iter()
            .zip(&current.ov)
            .any(|(a, b)| a.scale_factor != b.scale_factor),
        "DisturbanceVariables" => obj
            .dv
            .iter()
            .zip(&current.dv)
            .any(|(a, b)| a.scale_factor != b.scale_factor),
        "Model" => {
            !e.equal_arrays(&obj.plant_array, &current.plant_array)?
                || !e.equal_arrays(&obj.disturbance, &current.disturbance)?
                || !e.equal_arrays(&obj.noise, &current.noise)?
                || !mat_eq(&obj.nominal.x, &current.nominal.x)
                || !mat_eq(&obj.nominal.u, &current.nominal.u)
                || !mat_eq(&obj.nominal.y, &current.nominal.y)
                || !mat_eq(&obj.nominal.dx, &current.nominal.dx)
        }
        _ => false,
    };
    if reset_estimator {
        obj.estimator = super::estimator::EstimatorMode::Default;
    }
    Ok(obj)
}

fn mat_eq(a: &Matrix, b: &Matrix) -> bool {
    a.rows() == b.rows() && a.cols() == b.cols() && a.data() == b.data()
}

fn target_eq(a: &MvTarget, b: &MvTarget) -> bool {
    match (a, b) {
        (MvTarget::Scalar(x), MvTarget::Scalar(y)) => x == y,
        (MvTarget::Vector(x), MvTarget::Vector(y)) => mat_eq(x, y),
        (MvTarget::Nominal, MvTarget::Nominal) => true,
        _ => false,
    }
}

pub(in crate::engine) fn equal(
    a: &MpcObject,
    b: &MpcObject,
    mut eq_arr: impl FnMut(&Array, &Array) -> Result<bool>,
) -> Result<bool> {
    let mv_eq = a.mv.len() == b.mv.len()
        && a.mv.iter().zip(&b.mv).all(|(x, y)| {
            mat_eq(&x.min, &y.min)
                && mat_eq(&x.max, &y.max)
                && mat_eq(&x.rate_min, &y.rate_min)
                && mat_eq(&x.rate_max, &y.rate_max)
                && mat_eq(&x.min_ecr, &y.min_ecr)
                && mat_eq(&x.max_ecr, &y.max_ecr)
                && mat_eq(&x.rate_min_ecr, &y.rate_min_ecr)
                && mat_eq(&x.rate_max_ecr, &y.rate_max_ecr)
                && target_eq(&x.target, &y.target)
                && x.name == y.name
                && x.units == y.units
                && x.scale_factor == y.scale_factor
        });
    let ov_eq = a.ov.len() == b.ov.len()
        && a.ov.iter().zip(&b.ov).all(|(x, y)| {
            mat_eq(&x.min, &y.min)
                && mat_eq(&x.max, &y.max)
                && mat_eq(&x.min_ecr, &y.min_ecr)
                && mat_eq(&x.max_ecr, &y.max_ecr)
                && x.name == y.name
                && x.units == y.units
                && x.scale_factor == y.scale_factor
        });
    let weights_eq = mat_eq(&a.weights.mv, &b.weights.mv)
        && mat_eq(&a.weights.mv_rate, &b.weights.mv_rate)
        && mat_eq(&a.weights.ov, &b.weights.ov)
        && a.weights.ecr == b.weights.ecr;
    let nom_eq = mat_eq(&a.nominal.x, &b.nominal.x)
        && mat_eq(&a.nominal.u, &b.nominal.u)
        && mat_eq(&a.nominal.y, &b.nominal.y)
        && mat_eq(&a.nominal.dx, &b.nominal.dx);
    if a.ts != b.ts
        || a.prediction_horizon != b.prediction_horizon
        || a.control_horizon != b.control_horizon
        || !weights_eq
        || !mv_eq
        || !ov_eq
        || a.dv != b.dv
        || !nom_eq
        || a.optimizer != b.optimizer
        || a.mv_idx != b.mv_idx
        || a.md_idx != b.md_idx
        || a.ud_idx != b.ud_idx
        || a.mo_idx != b.mo_idx
        || a.uo_idx != b.uo_idx
        || !mat_eq(&a.ad, &b.ad)
        || !mat_eq(&a.bd, &b.bd)
        || !mat_eq(&a.cd, &b.cd)
        || !mat_eq(&a.dd, &b.dd)
    {
        return Ok(false);
    }
    Ok(eq_arr(&a.plant_array, &b.plant_array)?
        && eq_arr(&a.disturbance, &b.disturbance)?
        && eq_arr(&a.noise, &b.noise)?
        && eq_arr(&a.output_disturbance, &b.output_disturbance)?
        && match (&a.estimator, &b.estimator) {
            (
                super::estimator::EstimatorMode::Default,
                super::estimator::EstimatorMode::Default,
            )
            | (super::estimator::EstimatorMode::Custom, super::estimator::EstimatorMode::Custom) => {
                true
            }
            (
                super::estimator::EstimatorMode::Gains(al, am),
                super::estimator::EstimatorMode::Gains(bl, bm),
            ) => mat_eq(al, bl) && mat_eq(am, bm),
            _ => false,
        }
        && eq_arr(&a.notes, &b.notes)?
        && eq_arr(&a.user_data, &b.user_data)?)
}

pub(in crate::engine) fn get_method(e: &Engine, m: &MpcObject, rest: &[Value]) -> Result<Value> {
    match rest {
        [] => {
            let mut fields = Vec::with_capacity(PROPERTY_NAMES.len());
            for &name in PROPERTY_NAMES {
                let v = property(e, m, name)?.array(&e.cancel)?;
                fields.push((name, v));
            }
            Ok(Value::from(struct_arr(e, fields)?))
        }
        [prop] => {
            let a = prop.clone().array(&e.cancel)?;
            if let Some(name) = text_opt(e, &a) {
                property(e, m, &name)
            } else if let Array::Cell(cell) = a {
                let mut items = Vec::with_capacity(cell.len());
                for i in 0..cell.len() {
                    let pname = text_req(e, cell.get_linear(i)?, "get property name")?;
                    let v = property(e, m, &pname)?.array(&e.cancel)?;
                    items.push(Arc::new(v));
                }
                Ok(Value::from(Array::Cell(CellArray::from_shared(
                    1,
                    items.len(),
                    items,
                    &e.cancel,
                )?)))
            } else {
                Err("get: property name must be text or a cell array of text".into())
            }
        }
        _ => Err("get: expected at most two arguments".into()),
    }
}

pub(in crate::engine) fn set_method(
    e: &mut Engine,
    m: &MpcObject,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    if rest.is_empty() {
        let mut fields = Vec::new();
        for &name in PROPERTY_NAMES {
            if name == "History" {
                continue;
            }
            fields.push((
                name,
                Array::Cell(CellArray::from_shared(0, 0, Vec::new(), &e.cancel)?),
            ));
        }
        return Ok(Value::from(struct_arr(e, fields)?));
    }
    let mut next = m.clone();
    if rest.len() == 1 {
        let a = rest[0].clone().array(&e.cancel)?;
        let Array::Struct(s) = a else {
            return Err("set: single argument must be a scalar property structure".into());
        };
        if s.len() != 1 {
            return Err("set: property structure must be scalar".into());
        }
        for (i, fname) in s.field_names().iter().enumerate() {
            let val = s.get(i, 0)?.as_ref().clone();
            next = assign(e, &next, &[PathStep::Field(fname.clone())], val, out)?;
        }
    } else {
        if !rest.len().is_multiple_of(2) {
            return Err("set: property name/value arguments must come in pairs".into());
        }
        for pair in rest.as_chunks::<2>().0 {
            let pname_arr = pair[0].clone().array(&e.cancel)?;
            let pname = text_req(e, &pname_arr, "set property name")?;
            let val = pair[1].clone().array(&e.cancel)?;
            next = assign(e, &next, &[PathStep::Field(pname)], val, out)?;
        }
    }
    super::super::objects::mpc_new(e, next)
}

/// Signal metadata lives beside the LTI matrices; conversion is delegated to
/// control_lti while these MPC-only groups remain on the original plant value.
/// https://www.mathworks.com/help/mpc/ref/setmpcsignals.html
pub(super) fn set_signals(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.is_empty() || !(args.len() - 1).is_multiple_of(2) || count > 1 {
        return Err("setmpcsignals: expected plant and signal-type/index pairs, one output".into());
    }
    let plant = args[0].clone().array(&e.cancel)?;
    let lti = control_lti::decode(e, &plant)?.ok_or("setmpcsignals: plant must be an LTI model")?;
    let (_, b, c, _) = e.ctl_ss(&lti, out)?;
    if b.cols() > super::prediction::MAX_DIM || c.rows() > super::prediction::MAX_DIM {
        return Err("setmpcsignals: channel dimensions exceed 128".into());
    }
    let mut inputs = vec![None; b.cols()];
    let mut outputs = vec![None; c.rows()];
    let mut seen = std::collections::HashSet::new();
    for pair in args[1..].as_chunks::<2>().0 {
        let name = super::text(e, &pair[0]).ok_or("setmpcsignals: signal type must be text")?;
        let (input, cat) = match name.as_str() {
            "MV" | "ManipulatedVariables" => (true, 0),
            "MD" | "MeasuredDisturbances" => (true, 1),
            "UD" | "UnmeasuredDisturbances" => (true, 2),
            "MO" | "MeasuredOutputs" => (false, 0),
            "UO" | "UnmeasuredOutputs" => (false, 1),
            _ => return Err(format!("setmpcsignals: unknown signal type '{name}'")),
        };
        if !seen.insert((input, cat)) {
            return Err(format!("setmpcsignals: repeated signal type '{name}'"));
        }
        let target = if input { &mut inputs } else { &mut outputs };
        let value = pair[1].clone().array(&e.cancel)?;
        let v = real_matrix(&value, &name)?;
        if !v.is_empty() && v.rows() != 1 && v.cols() != 1 {
            return Err("setmpcsignals: indices must be a vector".into());
        }
        for &x in v.data() {
            if !x.is_finite() || x < 1. || x.fract() != 0. || x > target.len() as f64 {
                return Err(format!("setmpcsignals: {name} index out of range"));
            }
            let slot = &mut target[x as usize - 1];
            if slot.is_some() {
                return Err("setmpcsignals: channel indices must not overlap or repeat".into());
            }
            *slot = Some(cat);
        }
    }
    let groups = |names: &[&str], channels: Vec<Option<usize>>| -> Result<Array> {
        let mut fields = Vec::new();
        for (cat, &name) in names.iter().enumerate() {
            let idx: Vec<_> = channels
                .iter()
                .enumerate()
                .filter(|(_, c)| c.unwrap_or(0) == cat)
                .map(|(i, _)| (i + 1) as f64)
                .collect();
            fields.push((
                name,
                Array::Numeric(Matrix::from_column_vec(1, idx.len(), idx)?),
            ));
        }
        struct_arr(e, fields)
    };
    let Array::Struct(s) = plant else {
        unreachable!()
    };
    let result = s
        .with_field(
            "InputGroup",
            groups(&["MV", "MD", "UD"], inputs)?,
            &e.cancel,
        )?
        .with_field("OutputGroup", groups(&["MO", "UO"], outputs)?, &e.cancel)?;
    Ok(vec![Value::from(Array::Struct(result))])
}
