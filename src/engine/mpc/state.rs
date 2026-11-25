//! Validated MPC controller state handle; Plant and LastMove are absolute values.
use super::super::{Engine, Execution, Value};
use super::{estimator, object::MpcObject, prediction};
use crate::{Array, Matrix, Result};

pub(in crate::engine) const PROPERTIES: &[&str] =
    &["Plant", "Disturbance", "Noise", "LastMove", "Covariance"];
#[derive(Clone)]
pub(in crate::engine) struct MpcState {
    pub plant: Matrix,
    pub disturbance: Matrix,
    pub noise: Matrix,
    pub last_move: Matrix,
    pub covariance: Matrix,
    pub custom: bool,
    pub active: Vec<bool>,
    pub active_variables: usize,
}
impl MpcState {
    pub fn bytes(&self) -> usize {
        128 + self.active.len()
            + 8 * (self.plant.len()
                + self.disturbance.len()
                + self.noise.len()
                + self.last_move.len()
                + self.covariance.len())
    }
}
pub(in crate::engine) fn property(s: &MpcState, name: &str) -> Result<Value> {
    Ok(Value::Matrix(
        match name {
            "Plant" => &s.plant,
            "Disturbance" => &s.disturbance,
            "Noise" => &s.noise,
            "LastMove" => &s.last_move,
            "Covariance" => &s.covariance,
            _ => return Err(format!("mpcstate: unknown property '{name}'")),
        }
        .clone(),
    ))
}
pub(in crate::engine) fn assign(s: &MpcState, name: &str, value: &Array) -> Result<MpcState> {
    let Array::Numeric(v) = value else {
        return Err(format!("mpcstate: {name} must be real double"));
    };
    if v.is_nd() || v.data().iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "mpcstate: {name} must be finite and two-dimensional"
        ));
    }
    let mut next = s.clone();
    let target = match name {
        "Plant" => &mut next.plant,
        "Disturbance" => &mut next.disturbance,
        "Noise" => &mut next.noise,
        "LastMove" => &mut next.last_move,
        "Covariance" => &mut next.covariance,
        _ => return Err(format!("mpcstate: unknown property '{name}'")),
    };
    if name == "Covariance" {
        let n = s.plant.len() + s.disturbance.len() + s.noise.len();
        if !(s.custom && v.is_empty()) && (v.rows() != n || v.cols() != n) {
            return Err(format!("mpcstate: Covariance must be {n}-by-{n}"));
        }
        for j in 0..v.cols() {
            for i in 0..j {
                if (v.get(i, j) - v.get(j, i)).abs()
                    > 1e-12 * (1. + v.get(i, j).abs().max(v.get(j, i).abs()))
                {
                    return Err("mpcstate: Covariance must be symmetric".into());
                }
            }
        }
        *target = v.clone();
    } else {
        if v.len() != target.len() || (!v.is_empty() && v.rows() != 1 && v.cols() != 1) {
            return Err(format!(
                "mpcstate: {name} must be a vector of length {}",
                target.len()
            ));
        }
        *target = v.reshape(v.len(), 1)?;
    }
    Ok(next)
}
fn vector_summary(v: &Matrix) -> String {
    if v.len() > 8 {
        return format!("[{}x1 double]", v.len());
    }
    let values = v
        .data()
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    format!("[{values}]")
}
pub(in crate::engine) fn body(s: &MpcState) -> String {
    format!(
        "  MPCSTATE object with fields\n          Plant: {}\n    Disturbance: {}\n          Noise: {}\n       LastMove: {}\n     Covariance: [{}x{} double]\n",
        vector_summary(&s.plant),
        vector_summary(&s.disturbance),
        vector_summary(&s.noise),
        vector_summary(&s.last_move),
        s.covariance.rows(),
        s.covariance.cols()
    )
}
pub(super) fn construct(
    e: &mut Engine,
    m: &MpcObject,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.len() > 5 || count > 1 {
        return Err("mpcstate: expected controller[,plant,disturbance,noise,lastMove,covariance] and one output".into());
    }
    let p = prediction::build(e, m, out)?;
    let k = estimator::design(e, &p, &m.estimator, out)?;
    let mut s = MpcState {
        active: Vec::new(),
        active_variables: 0,
        plant: m.nominal.x.clone(),
        disturbance: Matrix::filled(
            p.input_disturbance_states.len() + p.output_disturbance_states.len(),
            1,
            0.,
        )?,
        noise: Matrix::filled(p.noise_states.len(), 1, 0.)?,
        last_move: Matrix::from_column_vec(
            m.n_mv(),
            1,
            m.mv_idx.iter().map(|&i| m.nominal.u.get(i, 0)).collect(),
        )?,
        covariance: k.covariance,
        custom: matches!(m.estimator, estimator::EstimatorMode::Custom),
    };
    for (j, v) in args.iter().enumerate() {
        let a = v.clone().array(&e.cancel)?;
        if !matches!(&a,Array::Numeric(m) if m.is_empty()) {
            s = assign(&s, PROPERTIES[j], &a)?;
        }
    }
    Ok(vec![super::super::objects::mpc_state_new(e, s)?])
}
