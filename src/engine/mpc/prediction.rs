//! Controller model from MathWorks' public "MPC Prediction Models":
//! "An integrator is added for each measured output in order of decreasing output weight."
//! Ties follow physical output order; reject a "violation of state observability".
//! Input integrators are considered first, with static unity fallback; output
//! integrators have zero fallback. No integrators are added on unmeasured outputs.
//! Default integrators are continuous 1/s under ZOH: B=Ts, not B=1.
//! https://www.mathworks.com/help/mpc/ref/mpc.setoutdist.html
//! https://www.mathworks.com/help/mpc/gs/mpc-modeling.html
//! "each with zero mean and identity covariance" (Controller State Estimation).
//! This includes independent noise on MV/MD as well as all stochastic models.
//! https://www.mathworks.com/help/mpc/ug/controller-state-estimation.html

use super::super::{Engine, Execution, Value, control_lti};
use super::object::MpcObject;
use crate::{Array, Matrix, Result};
use nalgebra::DMatrix;
use std::ops::Range;

pub(super) const MAX_DIM: usize = 128;

/// Discrete observer and prediction model, with immutable shared column-major matrices.
/// States: plant, input disturbance, output disturbance, measurement noise.
/// Rows: MO then UO. Input/output index maps refer to the original plant (zero based).
/// States are engineering-unit deviations from x_offset; signals are dimensionless
/// deviations. u_offset/input_scale are in original plant input order;
/// y_offset is in original plant output order, output_scale is in MO/UO order.
/// Reorder (Y-y_offset) into MO/UO order before dividing by output_scale.
/// x+ = A*x + Bu*u_mv + Bv*[v_md;1] + Bd*w; y=C*x+Dv*[v_md;1]+Dd*w.
/// The final Bv column is discretized nominal DX, including continuous-time ZOH.
/// Use c_prediction for noise-free output prediction (noise-state columns zero).
#[derive(Clone, Debug)]
pub struct MpcPredictionModel {
    pub a: Matrix,
    pub bu: Matrix,
    pub bv: Matrix,
    /// White-noise inputs in input-disturbance, output-disturbance, noise order.
    pub bd: Matrix,
    pub c: Matrix,
    pub c_prediction: Matrix,
    pub du: Matrix,
    pub dv: Matrix,
    pub dd: Matrix,
    pub cm: Matrix,
    pub dvm: Matrix,
    pub mv_idx: Vec<usize>,
    pub md_idx: Vec<usize>,
    pub ud_idx: Vec<usize>,
    pub mo_idx: Vec<usize>,
    pub uo_idx: Vec<usize>,
    pub plant_states: Range<usize>,
    pub input_disturbance_states: Range<usize>,
    pub output_disturbance_states: Range<usize>,
    pub noise_states: Range<usize>,
    pub input_integrator_channels: Vec<usize>,
    pub output_integrator_channels: Vec<usize>,
    pub x_offset: Matrix,
    pub u_offset: Matrix,
    pub y_offset: Matrix,
    pub input_scale: Matrix,
    pub output_scale: Matrix,
    pub ts: f64,
}

#[derive(Clone)]
pub(super) struct Model {
    pub a: Matrix,
    pub b: Matrix,
    pub c: Matrix,
    pub d: Matrix,
}

pub(super) fn dm(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}
pub(super) fn matrix(m: DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_vec(m.nrows(), m.ncols(), m.as_slice().to_vec())
}
fn zeros(r: usize, c: usize) -> Result<Matrix> {
    Matrix::filled(r, c, 0.)
}
fn gain(d: Matrix) -> Result<Model> {
    Ok(Model {
        a: zeros(0, 0)?,
        b: zeros(0, d.cols())?,
        c: zeros(d.rows(), 0)?,
        d,
    })
}
fn default_dist(rows: usize, channels: &[usize], input: bool, ts: f64) -> Result<Model> {
    let n = channels.len();
    let inputs = if input { rows } else { n };
    let mut b = DMatrix::zeros(n, inputs);
    let mut c = DMatrix::zeros(rows, n);
    let mut d = if input {
        DMatrix::identity(rows, rows)
    } else {
        DMatrix::zeros(rows, inputs)
    };
    for (j, &i) in channels.iter().enumerate() {
        b[(j, if input { i } else { j })] = ts;
        c[(i, j)] = 1.;
        if input {
            d[(i, i)] = 0.;
        }
    }
    Ok(Model {
        a: Matrix::identity(n, n)?,
        b: matrix(b)?,
        c: matrix(c)?,
        d: matrix(d)?,
    })
}

/// Validate dimensions immediately on assignment; observability is checked when
/// the complete internal model is built. [] means restore the default model.
pub(super) fn decode_model(
    e: &mut Engine,
    value: &Array,
    rows: usize,
    ts: f64,
    label: &str,
    out: &mut Execution,
) -> Result<Option<Model>> {
    if matches!(value, Array::Numeric(m) if m.is_empty()) {
        return Ok(None);
    }
    let model = if let Array::Numeric(d) = value {
        if d.is_nd()
            || d.rows() != rows
            || d.cols() == 0
            || d.cols() > MAX_DIM
            || d.data().iter().any(|v| !v.is_finite())
        {
            return Err(format!(
                "mpc: {label} must be a finite static gain with {rows} rows or an LTI model"
            ));
        }
        gain(d.clone())?
    } else {
        let s = control_lti::decode(e, value)?
            .ok_or_else(|| format!("mpc: {label} must be a static gain, LTI model or []"))?;
        s.require_delay_free(label)?;
        if s.ts > 0. && (s.ts - ts).abs() > 1e-12 * ts.max(1.) {
            return Err(format!(
                "mpc: {label} sample time mismatch; d2d resampling is not supported"
            ));
        }
        let (a, b, c, d) = e.ctl_ss(&s, out)?;
        if a.rows() > MAX_DIM || b.cols() > MAX_DIM || c.rows() != rows || b.cols() == 0 {
            return Err(format!(
                "mpc: {label} must have {rows} outputs, 1..128 noise inputs and at most 128 states"
            ));
        }
        if s.ts == 0. && a.rows() != 0 {
            let sd = e.ctl_sample(&s, ts, "zoh", false, out)?;
            let (a, b, c, d) = e.ctl_ss(&sd, out)?;
            Model { a, b, c, d }
        } else {
            Model { a, b, c, d }
        }
    };
    Ok(Some(model))
}

fn workspace(e: &Engine, n: usize, ny: usize) -> Result<()> {
    e.check_cancel()?;
    let bytes = 8 * (n * n * (ny + 40) + 16 * MAX_DIM * MAX_DIM);
    if bytes > e.limits.memory_bytes {
        return Err("mpc: prediction workspace exceeds memory limit".into());
    }
    Ok(())
}

fn observable(e: &Engine, a: &DMatrix<f64>, c: &DMatrix<f64>) -> Result<bool> {
    let n = a.nrows();
    workspace(e, n, c.nrows())?;
    if n == 0 {
        return Ok(true);
    }
    if c.nrows() == 0 {
        return Ok(false);
    }
    let mut o = DMatrix::zeros(n * c.nrows(), n);
    let mut ca = c.clone();
    for k in 0..n {
        e.check_cancel()?;
        o.view_mut((k * c.nrows(), 0), (c.nrows(), n))
            .copy_from(&ca);
        ca = &ca * a;
        if ca.iter().any(|v| !v.is_finite()) {
            return Err("mpc: observability calculation overflow".into());
        }
    }
    let s = o.svd(false, false).singular_values;
    let tol = 100. * f64::EPSILON * (n * c.nrows()).max(n) as f64 * s[0];
    e.check_cancel()?;
    Ok(s.iter().filter(|&&v| v > tol).count() == n)
}

fn put(dest: &mut DMatrix<f64>, r: usize, c: usize, src: &Matrix) {
    dest.view_mut((r, c), (src.rows(), src.cols()))
        .copy_from(&dm(src));
}

fn combine(
    m: &MpcObject,
    id: &Model,
    od: &Model,
    noise: &Model,
) -> Result<(DMatrix<f64>, DMatrix<f64>)> {
    let p = m.nx();
    let i = id.a.rows();
    let o = od.a.rows();
    let n = noise.a.rows();
    let mut a = DMatrix::zeros(p + i + o + n, p + i + o + n);
    put(&mut a, 0, 0, &m.ad);
    put(&mut a, p, p, &id.a);
    put(&mut a, p + i, p + i, &od.a);
    put(&mut a, p + i + o, p + i + o, &noise.a);
    let bpd = DMatrix::from_fn(p, m.ud_idx.len(), |r, j| {
        m.bd.get(r, m.ud_idx[j]) * m.dv[m.md_idx.len() + j].scale_factor
    });
    let dpd = DMatrix::from_fn(m.ny(), m.ud_idx.len(), |r, j| {
        m.dd.get(r, m.ud_idx[j]) * m.dv[m.md_idx.len() + j].scale_factor
    });
    a.view_mut((0, p), (p, i)).copy_from(&(&bpd * dm(&id.c)));
    let mut c = DMatrix::zeros(m.ny(), p + i + o + n);
    put(&mut c, 0, 0, &m.cd);
    c.view_mut((0, p), (m.ny(), i))
        .copy_from(&(dpd * dm(&id.c)));
    // Plant output is scaled; disturbance/noise models are already dimensionless.
    let order: Vec<_> = m.mo_idx.iter().chain(&m.uo_idx).copied().collect();
    let mut ordered = DMatrix::zeros(m.ny(), c.ncols());
    for (r, &physical) in order.iter().enumerate() {
        ordered
            .row_mut(r)
            .copy_from(&(c.row(physical) / m.ov[physical].scale_factor));
        for j in 0..o {
            ordered[(r, p + i + j)] = od.c.get(physical, j);
        }
        if r < m.mo_idx.len() {
            for j in 0..n {
                ordered[(r, p + i + o + j)] = noise.c.get(r, j);
            }
        }
    }
    Ok((a, ordered))
}

struct Parts {
    id: Model,
    od: Model,
    noise: Model,
    ich: Vec<usize>,
    och: Vec<usize>,
}
fn parts(e: &mut Engine, m: &MpcObject, out: &mut Execution) -> Result<Parts> {
    if m.nx() > MAX_DIM || m.nu() > MAX_DIM || m.ny() > MAX_DIM {
        return Err("mpc: model dimensions exceed 128".into());
    }
    let max_states = (m.nx() + m.nu() + 2 * m.ny()).min(MAX_DIM);
    let bytes = 8 * (max_states * max_states * (m.mo_idx.len() + 40) + 16 * MAX_DIM * MAX_DIM);
    if bytes > e.limits.memory_bytes {
        return Err("mpc: prediction workspace exceeds memory limit".into());
    }
    let mut id = decode_model(
        e,
        &m.disturbance,
        m.ud_idx.len(),
        m.ts,
        "Model.Disturbance",
        out,
    )?;
    let mut od = decode_model(
        e,
        &m.output_disturbance,
        m.ny(),
        m.ts,
        "output disturbance",
        out,
    )?;
    let mut noise = decode_model(e, &m.noise, m.mo_idx.len(), m.ts, "Model.Noise", out)?
        .unwrap_or(gain(Matrix::identity(m.mo_idx.len(), m.mo_idx.len())?)?);
    // Public documented regularization of a near singular measurement feedthrough.
    if noise.d.rows() != 0 {
        let d = dm(&noise.d);
        let eig = (&d * d.transpose()).symmetric_eigen().eigenvalues;
        if eig.min() < 1e-8 {
            let mut d = d;
            for i in 0..d.nrows().min(d.ncols()) {
                d[(i, i)] += 1e-4;
            }
            noise.d = matrix(d)?;
        }
    }
    let mut ich = Vec::new();
    let mut och = Vec::new();
    if id.is_none() {
        for j in 0..m.ud_idx.len() {
            e.check_cancel()?;
            let mut candidate = ich.clone();
            candidate.push(j);
            let im = default_dist(m.ud_idx.len(), &candidate, true, m.ts)?;
            let om = od
                .clone()
                .unwrap_or(default_dist(m.ny(), &[], false, m.ts)?);
            if m.nx() + im.a.rows() + om.a.rows() + noise.a.rows() > MAX_DIM {
                return Err("mpc: augmented states exceed 128".into());
            }
            let (a, c) = combine(m, &im, &om, &noise)?;
            if observable(e, &a, &c.rows(0, m.mo_idx.len()).into_owned())? {
                ich = candidate;
            }
        }
        id = Some(default_dist(m.ud_idx.len(), &ich, true, m.ts)?);
    }
    let id = id.unwrap();
    if od.is_none() {
        let mut order: Vec<usize> = (0..m.mo_idx.len()).collect();
        let weights: Vec<f64> = order
            .iter()
            .map(|&j| {
                (0..m.weights.ov.rows())
                    .map(|r| m.weights.ov.get(r, m.mo_idx[j]).abs())
                    .sum()
            })
            .collect();
        order.sort_by(|&i, &j| {
            weights[j]
                .total_cmp(&weights[i])
                .then(m.mo_idx[i].cmp(&m.mo_idx[j]))
        });
        for j in order {
            e.check_cancel()?;
            let mut candidate = och.clone();
            candidate.push(m.mo_idx[j]);
            let om = default_dist(m.ny(), &candidate, false, m.ts)?;
            if m.nx() + id.a.rows() + om.a.rows() + noise.a.rows() > MAX_DIM {
                return Err("mpc: augmented states exceed 128".into());
            }
            let (a, c) = combine(m, &id, &om, &noise)?;
            if observable(e, &a, &c.rows(0, m.mo_idx.len()).into_owned())? {
                och = candidate;
            }
        }
        od = Some(default_dist(m.ny(), &och, false, m.ts)?);
    }
    let od = od.unwrap();
    if m.nx() + id.a.rows() + od.a.rows() + noise.a.rows() > MAX_DIM {
        return Err("mpc: augmented states exceed 128".into());
    }
    Ok(Parts {
        id,
        od,
        noise,
        ich,
        och,
    })
}

pub(super) fn disturbance(
    e: &mut Engine,
    m: &MpcObject,
    input: bool,
    out: &mut Execution,
) -> Result<(Value, Vec<usize>)> {
    let p = parts(e, m, out)?;
    let (model, ch) = if input { (p.id, p.ich) } else { (p.od, p.och) };
    Ok((encode(e, &model, m.ts, out)?, ch))
}
pub(super) fn encode(e: &mut Engine, m: &Model, ts: f64, out: &mut Execution) -> Result<Value> {
    // The LTI engine rejects zero-port ss models. [] denotes an absent model;
    // internally it retains its exact zero-column stochastic input dimensions.
    if m.d.rows() == 0 || m.d.cols() == 0 {
        return Ok(Value::Matrix(zeros(0, 0)?));
    }
    e.ctl_call(
        "ss",
        &[
            Value::Matrix(m.a.clone()),
            Value::Matrix(m.b.clone()),
            Value::Matrix(m.c.clone()),
            Value::Matrix(m.d.clone()),
            Value::Matrix(Matrix::scalar(ts)),
        ],
        out,
    )?
    .ok_or("mpc: ss unavailable".into())
}

pub(in crate::engine) fn build(
    e: &mut Engine,
    m: &MpcObject,
    out: &mut Execution,
) -> Result<MpcPredictionModel> {
    let Parts {
        id,
        od,
        noise,
        ich,
        och,
    } = parts(e, m, out)?;
    workspace(
        e,
        m.nx() + id.a.rows() + od.a.rows() + noise.a.rows(),
        m.mo_idx.len(),
    )?;
    let (a, c) = combine(m, &id, &od, &noise)?;
    let nx = m.nx();
    let ni = id.a.rows();
    let no = od.a.rows();
    let nn = noise.a.rows();
    let n = a.nrows();
    let nd = id.b.cols() + od.b.cols() + noise.b.cols();
    let mut bu = DMatrix::zeros(n, m.n_mv());
    let mut bv = DMatrix::zeros(n, m.md_idx.len() + 1);
    let mut dv = DMatrix::zeros(m.ny(), m.md_idx.len() + 1);
    let mut bd = DMatrix::zeros(n, nd);
    let mut dd = DMatrix::zeros(m.ny(), nd);
    let mut scales = vec![1.; m.nu()];
    for (j, &i) in m.mv_idx.iter().enumerate() {
        scales[i] = m.mv[j].scale_factor;
    }
    for (j, &i) in m.md_idx.iter().chain(&m.ud_idx).enumerate() {
        scales[i] = m.dv[j].scale_factor;
    }
    for (j, &i) in m.mv_idx.iter().enumerate() {
        for r in 0..nx {
            bu[(r, j)] = m.bd.get(r, i) * scales[i];
        }
    }
    for (j, &i) in m.md_idx.iter().enumerate() {
        for r in 0..nx {
            bv[(r, j)] = m.bd.get(r, i) * scales[i];
        }
    }
    let dx = discrete_nominal_dx(e, m, out)?;
    for r in 0..nx {
        bv[(r, m.md_idx.len())] = dx.get(r, 0);
    }
    let bpd = DMatrix::from_fn(nx, m.ud_idx.len(), |r, j| {
        m.bd.get(r, m.ud_idx[j]) * scales[m.ud_idx[j]]
    });
    bd.view_mut((0, 0), (nx, id.b.cols()))
        .copy_from(&(bpd * dm(&id.d)));
    put(&mut bd, nx, 0, &id.b);
    put(&mut bd, nx + ni, id.b.cols(), &od.b);
    put(&mut bd, nx + ni + no, id.b.cols() + od.b.cols(), &noise.b);
    for (r, &i) in m.mo_idx.iter().chain(&m.uo_idx).enumerate() {
        let scale = m.ov[i].scale_factor;
        for (j, &v) in m.md_idx.iter().enumerate() {
            dv[(r, j)] = m.dd.get(i, v) * scales[v] / scale;
        }
        for j in 0..id.d.cols() {
            for k in 0..m.ud_idx.len() {
                dd[(r, j)] +=
                    m.dd.get(i, m.ud_idx[k]) * scales[m.ud_idx[k]] * id.d.get(k, j) / scale;
            }
        }
        for j in 0..od.d.cols() {
            dd[(r, id.d.cols() + j)] = od.d.get(i, j);
        }
        if r < m.mo_idx.len() {
            for j in 0..noise.d.cols() {
                dd[(r, id.d.cols() + od.d.cols() + j)] = noise.d.get(r, j);
            }
        }
    }
    if !matches!(m.estimator, super::estimator::EstimatorMode::Custom)
        && !observable(e, &a, &c.rows(0, m.mo_idx.len()).into_owned())?
    {
        return Err(
            "mpc: augmented controller model is not observable from measured outputs".into(),
        );
    }
    let mut cp = c.clone();
    cp.view_mut((0, nx + ni + no), (m.ny(), nn)).fill(0.);
    let cm = matrix(c.rows(0, m.mo_idx.len()).into_owned())?;
    let dvm = matrix(dv.rows(0, m.mo_idx.len()).into_owned())?;
    Ok(MpcPredictionModel {
        a: matrix(a)?,
        bu: matrix(bu)?,
        bv: matrix(bv)?,
        bd: matrix(bd)?,
        c: matrix(c)?,
        c_prediction: matrix(cp)?,
        du: zeros(m.ny(), m.n_mv())?,
        dv: matrix(dv)?,
        dd: matrix(dd)?,
        cm,
        dvm,
        mv_idx: m.mv_idx.clone(),
        md_idx: m.md_idx.clone(),
        ud_idx: m.ud_idx.clone(),
        mo_idx: m.mo_idx.clone(),
        uo_idx: m.uo_idx.clone(),
        plant_states: 0..nx,
        input_disturbance_states: nx..nx + ni,
        output_disturbance_states: nx + ni..nx + ni + no,
        noise_states: nx + ni + no..n,
        input_integrator_channels: ich,
        output_integrator_channels: och,
        x_offset: m.nominal.x.clone(),
        u_offset: m.nominal.u.clone(),
        y_offset: m.nominal.y.clone(),
        input_scale: Matrix::from_column_vec(m.nu(), 1, scales)?,
        output_scale: Matrix::from_column_vec(
            m.ny(),
            1,
            m.mo_idx
                .iter()
                .chain(&m.uo_idx)
                .map(|&i| m.ov[i].scale_factor)
                .collect(),
        )?,
        ts: m.ts,
    })
}
pub(super) fn discrete_nominal_dx(
    e: &mut Engine,
    m: &MpcObject,
    out: &mut Execution,
) -> Result<Matrix> {
    let plant = control_lti::decode(e, &m.plant_array)?.ok_or("mpc: invalid plant")?;
    if plant.ts != 0. || m.nominal.dx.data().iter().all(|&x| x == 0.) {
        return Ok(m.nominal.dx.clone());
    }
    let (a, b, c, d) = e.ctl_ss(&plant, out)?;
    let b = Matrix::concatenate(vec![vec![b, m.nominal.dx.clone()]])?;
    let d = Matrix::concatenate(vec![vec![d, zeros(c.rows(), 1)?]])?;
    let s = encode(e, &Model { a, b, c, d }, 0., out)?.array(&e.cancel)?;
    let s = control_lti::decode(e, &s)?.ok_or("mpc: invalid affine model")?;
    let sd = e.ctl_sample(&s, m.ts, "zoh", false, out)?;
    let (_, b, _, _) = e.ctl_ss(&sd, out)?;
    Matrix::from_column_vec(
        m.nx(),
        1,
        (0..m.nx()).map(|r| b.get(r, b.cols() - 1)).collect(),
    )
}
