//! Steady-state Kalman design using the engine's DARE (no duplicate solver).
use super::super::{Engine, Execution, Value, control_design};
use super::{
    object::MpcObject,
    prediction::{self, MpcPredictionModel},
};
use crate::{Array, Matrix, Result};

#[derive(Clone, Debug, Default)]
pub(in crate::engine) enum EstimatorMode {
    #[default]
    Default,
    Gains(Matrix, Matrix),
    Custom,
}

#[derive(Clone, Debug)]
pub struct MpcEstimator {
    pub l: Matrix,
    pub m: Matrix,
    /// A-priori steady-state error covariance (DARE solution).
    pub covariance: Matrix,
}

pub(in crate::engine) fn design(
    e: &mut Engine,
    model: &MpcPredictionModel,
    mode: &EstimatorMode,
    out: &mut Execution,
) -> Result<MpcEstimator> {
    if matches!(mode, EstimatorMode::Custom) {
        return Ok(MpcEstimator {
            l: Matrix::filled(0, 0, 0.)?,
            m: Matrix::filled(0, 0, 0.)?,
            covariance: Matrix::filled(0, 0, 0.)?,
        });
    }
    let n = model.a.rows();
    let ny = model.cm.rows();
    if n == 0 {
        return Ok(MpcEstimator {
            l: Matrix::filled(0, ny, 0.)?,
            m: Matrix::filled(0, ny, 0.)?,
            covariance: Matrix::filled(0, 0, 0.)?,
        });
    }
    if ny == 0 {
        return Err("mpc: default estimator requires measured outputs".into());
    }
    let b = Matrix::concatenate(vec![vec![
        model.bu.clone(),
        prediction::matrix(
            prediction::dm(&model.bv)
                .columns(0, model.md_idx.len())
                .into_owned(),
        )?,
        model.bd.clone(),
    ]])?;
    let d = Matrix::concatenate(vec![vec![
        model.du.clone(),
        prediction::matrix(
            prediction::dm(&model.dv)
                .columns(0, model.md_idx.len())
                .into_owned(),
        )?,
        model.dd.clone(),
    ]])?;
    let d = prediction::matrix(prediction::dm(&d).rows(0, ny).into_owned())?;
    let q = b.mul(&b.transpose())?;
    let r = d.mul(&d.transpose())?;
    let cross = b.mul(&d.transpose())?;
    let result = control_design::cds_call(
        e,
        "dare",
        &[
            Value::Matrix(model.a.transpose()),
            Value::Matrix(model.cm.transpose()),
            Value::Matrix(q),
            Value::Matrix(r.clone()),
            Value::Matrix(cross.clone()),
        ],
        1,
        out,
    )?;
    let Value::Matrix(p) = &result[0] else {
        return Err("mpc: DARE returned non-real covariance".into());
    };
    let pc = p.mul(&model.cm.transpose())?;
    let innovation = model.cm.mul(&pc)?.zip(&r, |a, b| a + b)?;
    let m = innovation.solve(&pc.transpose())?.transpose();
    let apc = model.a.mul(&pc)?.zip(&cross, |a, b| a + b)?;
    let l = innovation.solve(&apc.transpose())?.transpose();
    e.check_cancel()?;
    let (l, m) = match mode {
        EstimatorMode::Gains(l, m) => {
            validate_gain(l, n, ny, "L")?;
            validate_gain(m, n, ny, "M")?;
            (l.clone(), m.clone())
        }
        _ => (l, m),
    };
    Ok(MpcEstimator {
        l,
        m,
        covariance: p.clone(),
    })
}
fn validate_gain(v: &Matrix, n: usize, ny: usize, label: &str) -> Result<()> {
    if v.is_nd() || v.rows() != n || v.cols() != ny || v.data().iter().any(|v| !v.is_finite()) {
        return Err(format!(
            "setEstimator: {label} must be a finite {n}-by-{ny} matrix"
        ));
    }
    Ok(())
}
fn gain_arg(v: Option<&Value>, n: usize, ny: usize, label: &str) -> Result<Option<Matrix>> {
    match v {
        None => Ok(None),
        Some(Value::Matrix(v)) if v.is_empty() => Ok(None),
        Some(Value::Matrix(v)) => {
            validate_gain(v, n, ny, label)?;
            Ok(Some(v.clone()))
        }
        _ => Err(format!(
            "setEstimator: {label} must be a real double matrix"
        )),
    }
}

pub(in crate::engine) fn setter(
    e: &mut Engine,
    m: &MpcObject,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    let mut next = m.clone();
    if name == "setEstimator" {
        if args.len() == 1
            && let Some(mode) = super::text(e, &args[0])
        {
            next.estimator = match mode.as_str() {
                "default" => EstimatorMode::Default,
                "custom" => EstimatorMode::Custom,
                _ => return Err("setEstimator: mode must be 'default' or 'custom'".into()),
            };
        } else {
            if args.is_empty() || args.len() > 2 {
                return Err(
                    "setEstimator expects controller,L[,M] or controller,'default'/'custom'".into(),
                );
            }
            let p = prediction::build(e, m, out)?;
            let n = p.a.rows();
            let ny = p.cm.rows();
            let mg = gain_arg(args.get(1), n, ny, "M")?.unwrap_or(Matrix::filled(n, ny, 0.)?);
            let lg = gain_arg(args.first(), n, ny, "L")?.unwrap_or(p.a.mul(&mg)?);
            next.estimator = EstimatorMode::Gains(lg, mg);
        }
    } else {
        let input = name == "setindist";
        let value = match args {
            [mode] if super::text(e, mode).as_deref() == Some("integrators") => {
                Array::Numeric(Matrix::filled(0, 0, 0.)?)
            }
            [mode] if super::text(e, mode).as_deref() == Some("model") => {
                Array::Numeric(Matrix::filled(0, 0, 0.)?)
            }
            [mode, v] if super::text(e, mode).as_deref() == Some("model") => {
                v.clone().array(&e.cancel)?
            }
            _ => {
                return Err(format!(
                    "{name}: expected controller,'model',model or controller,'integrators'"
                ));
            }
        };
        prediction::decode_model(
            e,
            &value,
            if input { m.ud_idx.len() } else { m.ny() },
            m.ts,
            if input {
                "Model.Disturbance"
            } else {
                "output disturbance"
            },
            out,
        )?;
        if input {
            next.disturbance = value;
        } else {
            next.output_disturbance = value;
        }
        next.estimator = EstimatorMode::Default;
    }
    super::super::objects::mpc_new(e, next)
}

pub(super) fn outputs(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let m = args
        .first()
        .and_then(|v| super::super::objects::mpc_value(e, v))
        .ok_or_else(|| format!("{name}: first input must be an mpc controller"))?;
    match name {
        "setEstimator" | "setindist" | "setoutdist" => {
            if count > 1 {
                return Err(format!("{name}: too many outputs"));
            }
            Ok(vec![setter(e, &m, name, &args[1..], out)?])
        }
        "getindist" | "getoutdist" => {
            if args.len() != 1 || count > 2 {
                return Err(format!(
                    "{name}: expected one input and at most two outputs"
                ));
            }
            let (sys, ch) = prediction::disturbance(e, &m, name == "getindist", out)?;
            Ok(vec![
                sys,
                Value::Matrix(Matrix::from_column_vec(
                    1,
                    ch.len(),
                    ch.iter().map(|&i| (i + 1) as f64).collect(),
                )?),
            ])
        }
        "getEstimator" => {
            if args.len() != 1 || count > 7 {
                return Err("getEstimator: expected one input and at most seven outputs; 'sys' form is not supported yet".into());
            }
            let p = prediction::build(e, &m, out)?;
            let k = design(e, &p, &m.estimator, out)?;
            Ok(vec![k.l, k.m, p.a, p.cm, p.bu, p.bv, p.dvm]
                .into_iter()
                .map(Value::Matrix)
                .collect())
        }
        "mpcstate" => super::state::construct(e, &m, &args[1..], count, out),
        _ => Err(format!("mpc: unsupported function {name}")),
    }
}
