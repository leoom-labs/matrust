//! Initial value solvers with engine-independent problem callbacks (`Model`).
//! Mathematical references: Shampine & Gordon, Computer Solution of Ordinary
//! Differential Equations (1975), Adams methods (ode113); Shampine & Reichelt,
//! The MATLAB ODE Suite (1997), sections 2–4, NDF/BDF (ode15s) and modified
//! Rosenbrock formulas (ode23s); Shampine, Reichelt & Kierzenka, Solving Index-1
//! DAEs in MATLAB and Simulink (1999), sections 3–4 (mass matrices); Hairer &
//! Wanner, Solving Ordinary Differential Equations II (1996), sections IV.8 and
//! VI.1, implicit methods; Bank et al., Transient Simulation of Silicon Devices
//! and Circuits (1985), trapezoidal/BDF composition (ode23tb). The trapezoidal
//! rule underlies ode23t. Adaptive meshes and statistics are implementation choices.
#![allow(clippy::needless_range_loop)]
use super::num::{Lu, R, eps_of, matvec, mmax, mmin, norm_inf, norm2, numjac, odezero, sign};

/// f(t, y) through the model, as the implicit solvers call it.
type FCall<'a> = dyn FnMut(&mut dyn Model, f64, &[f64]) -> R<Vec<f64>> + 'a;

/// The functions of an initial value problem, supplied by the engine.
pub trait Model {
    fn f(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>>;
    /// Analytic Jacobian (column-major neq-by-neq) when the Jacobian option is a function.
    fn jac(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>>;
    /// Mass matrix M(t, y) or M(t) when the Mass option is a function.
    fn mass(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>>;
    /// Event values, terminal flags and directions.
    fn events(&mut self, t: f64, y: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)>;
    fn check_cancel(&self) -> R<()>;
}

#[derive(Clone)]
pub enum JacKind {
    Numeric,
    Matrix(Vec<f64>),
    Function,
}

#[derive(Clone)]
pub enum MassKind {
    None,
    Matrix(Vec<f64>),
    /// M(t) (MStateDependence 'none') or M(t, y).
    Function {
        state: bool,
    },
}

#[derive(Clone)]
pub struct Opts {
    pub rtol: f64,
    /// Expanded to neq entries, including with NormControl.
    pub atol: Vec<f64>,
    pub normcontrol: bool,
    pub threshold: Vec<f64>,
    pub hmax: f64,
    pub htry: Option<f64>,
    pub htspan: f64,
    pub maxorder: usize,
    pub bdf: bool,
    pub jac: JacKind,
    pub jconstant: bool,
    pub mass: MassKind,
    /// MassSingular: "yes", "no" or "maybe".
    pub mass_singular: String,
    pub initial_slope: Option<Vec<f64>>,
    pub events: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum OutAt {
    Sol,
    Steps,
    Refined(usize),
    Requested,
}

/// Interpolation data of one accepted step, as MATLAB keeps it in sol.idata.
#[derive(Clone)]
pub enum StepData {
    /// ode113: order, modified divided differences phi (neq-by-14) and psi (12).
    Abm {
        k: usize,
        phi: Vec<Vec<f64>>,
        psi: Vec<f64>,
    },
    /// ode15s: order and backward differences dif (neq-by-maxk+2).
    Ndf { k: usize, dif: Vec<Vec<f64>> },
    /// ode23s stages.
    Ros { k1: Vec<f64>, k2: Vec<f64> },
    /// ode23t scaled derivatives.
    Tr { z: Vec<f64>, znew: Vec<f64> },
    /// ode23tb intermediate point.
    TrBdf { t2: f64, y2: Vec<f64> },
}

pub struct Out {
    pub t: Vec<f64>,
    pub y: Vec<Vec<f64>>,
    pub data: Vec<StepData>,
    pub data_dropped: bool,
    data_elements: usize,
    pub te: Vec<f64>,
    pub ye: Vec<Vec<f64>>,
    pub ie: Vec<usize>,
    /// nsteps, nfailed, nfevals, npds, ndecomps, nsolves.
    pub stats: [usize; 6],
    /// Time and smallest step when the tolerances could not be met.
    pub failure: Option<(f64, f64)>,
    /// Rows of the console text the solver printed (warnings).
    pub warnings: Vec<String>,
}

pub struct Setup<'a> {
    pub tspan: &'a [f64],
    pub y0: &'a [f64],
    pub f0: Vec<f64>,
    /// Initial event values, when there is an event function.
    pub valt: Option<Vec<f64>>,
    /// M(t0, y0) for a mass function, already evaluated during setup.
    pub m0: Option<Vec<f64>>,
    pub at: OutAt,
}

/// Bounded output: the solvers stop with an error past this many stored values.
pub const MAX_VALUES: usize = 32 * 1024 * 1024;

impl Out {
    fn new() -> Out {
        Out {
            t: Vec::new(),
            y: Vec::new(),
            data: Vec::new(),
            data_dropped: false,
            data_elements: 0,
            te: Vec::new(),
            ye: Vec::new(),
            ie: Vec::new(),
            stats: [0; 6],
            failure: None,
            warnings: Vec::new(),
        }
    }
    fn push(&mut self, t: f64, y: Vec<f64>) -> R<()> {
        if (self.t.len() + 1).saturating_mul(y.len().max(1) + 1) > MAX_VALUES {
            return Err(format!(
                "ODE solution exceeds the {MAX_VALUES}-element limit; shorten tspan or request fewer output times"
            ));
        }
        self.t.push(t);
        self.y.push(y);
        Ok(())
    }
    /// Output after an accepted step from t to tnew, as the odeXX output switch.
    #[allow(clippy::too_many_arguments)]
    fn step_output(
        &mut self,
        at: OutAt,
        tspan: &[f64],
        next: &mut usize,
        tdir: f64,
        t: f64,
        tnew: f64,
        ynew: &[f64],
        stop: bool,
        data: Option<StepData>,
        ntrp: &mut dyn FnMut(f64) -> Vec<f64>,
    ) -> R<()> {
        match at {
            OutAt::Sol => {
                self.push(tnew, ynew.to_vec())?;
                if let Some(d) = data {
                    self.push_data(d, 16 * 1024 * 1024);
                }
            }
            OutAt::Steps => self.push(tnew, ynew.to_vec())?,
            OutAt::Refined(refine) => {
                for i in 1..refine {
                    let s = i as f64 / refine as f64;
                    let tref = t + (tnew - t) * s;
                    let yref = ntrp(tref);
                    self.push(tref, yref)?;
                }
                self.push(tnew, ynew.to_vec())?;
            }
            OutAt::Requested => {
                while *next < tspan.len() {
                    let tout = tspan[*next];
                    if tdir * (tnew - tout) < 0. {
                        if stop {
                            self.push(tnew, ynew.to_vec())?;
                        }
                        break;
                    }
                    if tout == tnew {
                        self.push(tout, ynew.to_vec())?;
                    } else {
                        let yout = ntrp(tout);
                        self.push(tout, yout)?;
                    }
                    *next += 1;
                }
            }
        }
        Ok(())
    }
    fn add_events(&mut self, found: &super::num::Located) {
        self.te.extend_from_slice(&found.t);
        self.ye.extend(found.y.iter().cloned());
        self.ie.extend_from_slice(&found.i);
    }

    fn push_data(&mut self, data: StepData, limit: usize) {
        if self.data_dropped {
            return;
        }
        let elements = match &data {
            StepData::Abm { phi, psi, .. } => {
                phi.iter().map(Vec::len).sum::<usize>() + psi.len() + 1
            }
            StepData::Ndf { dif, .. } => dif.iter().map(Vec::len).sum::<usize>() + 1,
            StepData::Ros { k1, k2 } => k1.len() + k2.len(),
            StepData::Tr { z, znew } => z.len() + znew.len(),
            StepData::TrBdf { y2, .. } => y2.len() + 1,
        };
        if self.data_elements.saturating_add(elements) > limit {
            self.data = Vec::new();
            self.data_elements = 0;
            self.data_dropped = true;
        } else {
            self.data_elements += elements;
            self.data.push(data);
        }
    }
}

/// Weights max(|y|, threshold) (or the scalar NormControl weight).
fn weights(o: &Opts, y: &[f64]) -> Vec<f64> {
    if o.normcontrol {
        return vec![mmax(norm2(y), o.threshold[0]); y.len()];
    }
    (0..y.len())
        .map(|i| mmax(y[i].abs(), o.threshold[i]))
        .collect()
}

/// norm(v ./ wt, inf), or norm(v) / wt with NormControl.
fn wnorm(o: &Opts, v: &[f64], invwt: &[f64]) -> f64 {
    if o.normcontrol {
        norm2(v) * invwt[0]
    } else {
        norm_inf((0..v.len()).map(|i| v[i] * invwt[i]))
    }
}

fn failure_warning(solver: &str, t: f64, hmin: f64) -> String {
    let _ = solver;
    format!(
        "Warning: Integration stopped at t={}; meeting the tolerances would require a step below the minimum ({}).",
        format_e(t),
        format_e(hmin)
    )
}

/// C printf "%e", as MATLAB's sprintf('%e', x).
pub fn format_e(x: f64) -> String {
    if !x.is_finite() {
        return if x.is_nan() {
            "NaN".into()
        } else if x > 0. {
            "Inf".into()
        } else {
            "-Inf".into()
        };
    }
    let text = format!("{x:.6e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}

fn sub(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len()).map(|i| a[i] - b[i]).collect()
}

fn add(a: &[f64], b: &[f64]) -> Vec<f64> {
    (0..a.len()).map(|i| a[i] + b[i]).collect()
}

fn scale(a: f64, x: &[f64]) -> Vec<f64> {
    x.iter().map(|v| a * v).collect()
}

/// Integral of the Newton derivative polynomial, anchored at the accepted endpoint.
/// Hairer, Norsett & Wanner, Solving Ordinary Differential Equations I (1993),
/// III.5, equations (5.1)-(5.2). Arrays retain their public one-based layout:
/// phi[j+1] is the jth divided difference times psi[1] ... psi[j].
pub fn ntrp113(
    tinterp: f64,
    tnew: f64,
    ynew: &[f64],
    klast: usize,
    phi: &[Vec<f64>],
    psi: &[f64],
) -> Vec<f64> {
    let delta = tinterp - tnew;
    let coefficients = adams113_integrals(delta, klast, psi);
    adams113_sum(ynew, phi, &coefficients[..=klast])
}

/// Translate the same derivative polynomial when an event cuts an accepted step.
/// This is a change of Newton basis, not a new integration step (HNW, III.5).
fn rebase113(delta: f64, k: usize, phi: &mut [Vec<f64>], psi: &mut [f64]) {
    if delta == 0. {
        return;
    }
    let scale = psi[1];
    // Each row contains one Newton basis polynomial in z = (t - te)/scale.
    let mut old_basis = [[0.; 13]; 13];
    let mut new_basis = [[0.; 13]; 13];
    old_basis[0][0] = 1.;
    new_basis[0][0] = 1.;
    for j in 1..=k {
        let old_offset = if j == 1 { delta } else { delta + psi[j - 1] };
        let new_offset = if j == 1 { 0. } else { delta + psi[j - 1] };
        for r in 0..j {
            old_basis[j][r] += old_basis[j - 1][r] * (old_offset / psi[j]);
            old_basis[j][r + 1] += old_basis[j - 1][r] * (scale / psi[j]);
            new_basis[j][r] += new_basis[j - 1][r] * (new_offset / (psi[j] + delta));
            new_basis[j][r + 1] += new_basis[j - 1][r] * (scale / (psi[j] + delta));
        }
    }
    for i in 0..phi[1].len() {
        let mut polynomial = [0.; 13];
        for j in 0..=k {
            for r in 0..=j {
                polynomial[r] += phi[j + 1][i] * old_basis[j][r];
            }
        }
        for j in (0..=k).rev() {
            let coefficient = polynomial[j] / new_basis[j][j];
            phi[j + 1][i] = coefficient;
            for r in 0..j {
                polynomial[r] -= coefficient * new_basis[j][r];
            }
        }
    }
    for distance in &mut psi[1..=k] {
        *distance += delta;
    }
}

/// Integrals of the scaled Newton basis over [0, delta]. Form coefficients in
/// s/delta so powers of an absolute time never enter the calculation.
fn adams113_integrals(delta: f64, degree: usize, psi: &[f64]) -> [f64; 13] {
    let mut integrals = [0.; 13];
    if delta == 0. {
        return integrals;
    }
    let mut polynomial = [0.; 13];
    polynomial[0] = 1.;
    integrals[0] = delta;
    for j in 1..=degree {
        let a = if j == 1 { 0. } else { psi[j - 1] / psi[j] };
        let b = delta / psi[j];
        for r in (1..=j).rev() {
            polynomial[r] = a * polynomial[r] + b * polynomial[r - 1];
        }
        polynomial[0] *= a;
        integrals[j] = delta
            * polynomial[..=j]
                .iter()
                .enumerate()
                .map(|(r, c)| c / (r + 1) as f64)
                .sum::<f64>();
    }
    integrals
}

fn adams113_sum(y: &[f64], phi: &[Vec<f64>], coefficients: &[f64]) -> Vec<f64> {
    let mut result = vec![0.; y.len()];
    adams113_chunks(
        &mut result,
        y.len().saturating_mul(coefficients.len()) >= ADAMS113_PARALLEL_WORK,
        |start, chunk| {
            for j in (0..coefficients.len()).rev() {
                for (value, term) in chunk.iter_mut().zip(&phi[j + 1][start..]) {
                    *value += coefficients[j] * term;
                }
            }
            for (value, initial) in chunk.iter_mut().zip(&y[start..]) {
                *value += initial;
            }
        },
    );
    result
}

// Release measurements on an i7-12700KF favored serial work at 32K components
// and coarse blocks at 128K. Require roughly a million component/order pairs
// so low-order startup also stays serial (tool/cleanroom-ode113).
const ADAMS113_PARALLEL_WORK: usize = 1 << 20;

/// Reuse the caller's bounded pool, with coarse component blocks. Small vector
/// operations remain serial; no pool is created inside an integration step.
fn adams113_chunks(values: &mut [f64], parallel: bool, apply: impl Fn(usize, &mut [f64]) + Sync) {
    if parallel && crate::parallel::current_worker_count() > 1 {
        use rayon::prelude::*;
        values
            .par_chunks_mut(4096)
            .enumerate()
            .for_each(|(j, chunk)| apply(j * 4096, chunk));
    } else {
        apply(0, values);
    }
}

/// Insert the new derivative into the modified divided-difference table.
/// Multiplying the divided-difference recurrence by the cumulative node
/// distances gives new[j+1] = new[j] - ratio[j-1]*old[j] (HNW, III.5).
#[cfg(test)]
fn adams113_update(
    derivative: Vec<f64>,
    phi: &[Vec<f64>],
    psi: &[f64],
    h: f64,
    degree: usize,
) -> Vec<Vec<f64>> {
    let mut updated = Vec::new();
    adams113_update_into(derivative, phi, psi, h, degree, &mut updated);
    updated
}

fn adams113_update_into(
    derivative: Vec<f64>,
    phi: &[Vec<f64>],
    psi: &[f64],
    h: f64,
    degree: usize,
    updated: &mut Vec<Vec<f64>>,
) {
    let n = derivative.len();
    if updated.is_empty() {
        updated.push(Vec::new());
        updated.push(derivative);
    } else {
        updated[1] = derivative;
    }
    updated.resize_with(degree + 2, || vec![0.; n]);
    let mut ratios = [1.; 12];
    for j in 1..=degree {
        if j < degree {
            ratios[j] = ratios[j - 1] * (h + psi[j - 1]) / psi[j];
        }
    }
    if n.saturating_mul(degree) >= ADAMS113_PARALLEL_WORK
        && crate::parallel::current_worker_count() > 1
    {
        use rayon::prelude::*;
        // Each task advances all orders of a local component block. This
        // avoids a parallel dispatch and cross-worker traffic at every order.
        let mut columns: Vec<_> = updated[1..]
            .iter_mut()
            .map(|c| c.chunks_mut(4096))
            .collect();
        let blocks: Vec<Vec<&mut [f64]>> = (0..n.div_ceil(4096))
            .map(|_| columns.iter_mut().map(|c| c.next().unwrap()).collect())
            .collect();
        blocks
            .into_par_iter()
            .enumerate()
            .for_each(|(block, mut columns)| {
                adams113_update_block(&mut columns, phi, &ratios, block * 4096);
            });
    } else {
        let mut columns: Vec<_> = updated[1..].iter_mut().map(Vec::as_mut_slice).collect();
        adams113_update_block(&mut columns, phi, &ratios, 0);
    }
}

fn adams113_update_block(
    columns: &mut [&mut [f64]],
    phi: &[Vec<f64>],
    ratios: &[f64],
    start: usize,
) {
    for j in 1..columns.len() {
        let (previous, current) = columns.split_at_mut(j);
        for (i, value) in current[0].iter_mut().enumerate() {
            *value = previous[j - 1][i] - ratios[j - 1] * phi[j][start + i];
        }
    }
}

/// Abstract mass matrix handling shared by the solvers.
struct MassState {
    /// 0 none, 1 constant, 2 M(t), 3 M(t, y) (weak state dependence).
    mtype: u8,
    /// Current M (column-major); empty means the implicit identity.
    mt: Vec<f64>,
}

impl MassState {
    fn new(o: &Opts, _neq: usize, m0: Option<Vec<f64>>) -> MassState {
        match &o.mass {
            MassKind::None => MassState {
                mtype: 0,
                mt: Vec::new(),
            },
            MassKind::Matrix(m) => MassState {
                mtype: 1,
                mt: m.clone(),
            },
            MassKind::Function { state } => MassState {
                mtype: if *state { 3 } else { 2 },
                mt: m0.unwrap_or_default(),
            },
        }
    }
    fn eval(&self, m: &mut dyn Model, t: f64, y: &[f64]) -> R<Vec<f64>> {
        m.mass(t, y)
    }
    fn times(&self, v: &[f64]) -> Vec<f64> {
        if self.mtype == 0 {
            v.to_vec()
        } else {
            matvec(&self.mt, v.len(), v)
        }
    }
}

pub fn identity(n: usize) -> Vec<f64> {
    let mut m = vec![0.; n * n];
    for i in 0..n {
        m[i * n + i] = 1.;
    }
    m
}

/// Variable-step Adams PECE with orders 1 through 12 and local extrapolation.
/// Shampine & Gordon, Computer Solution of Ordinary Differential Equations:
/// The Initial Value Problem (1975), Adams predictor-corrector construction;
/// Hairer, Norsett & Wanner (1993), III.5 (modified divided differences) and
/// III.7 (local-error based step/order selection). The controller below is an
/// independent choice of safety factors and order-change hysteresis.
pub fn ode113(m: &mut dyn Model, s: Setup, o: &Opts) -> R<Out> {
    let mut out = Out::new();
    let t0 = s.tspan[0];
    let tf = *s.tspan.last().unwrap();
    let direction = sign(tf - t0);
    let n = s.y0.len();
    // History, trial tables and vectors are all O(n), including identity mass.
    if n.saturating_mul(80) > MAX_VALUES {
        return Err(format!(
            "ODE workspace exceeds the {MAX_VALUES}-element limit."
        ));
    }
    m.check_cancel()?;
    let mass = MassState::new(o, n, s.m0);
    let factor = (mass.mtype != 0).then(|| Lu::new(&mass.mt, n));
    let f0 = if let Some(lu) = &factor {
        lu.solve(&s.f0)
    } else {
        s.f0
    };
    // Cache constant mass factors; a functional mass is evaluated at each RHS.
    let rhs = |model: &mut dyn Model, time: f64, state: &[f64]| -> R<Vec<f64>> {
        model.check_cancel()?;
        let value = model.f(time, state)?;
        Ok(match mass.mtype {
            0 => value,
            1 => factor.as_ref().unwrap().solve(&value),
            _ => Lu::new(&mass.eval(model, time, state)?, n).solve(&value),
        })
    };
    out.stats[2] = 1;
    let mut t = t0;
    let mut y = s.y0.to_vec();
    let mut phi = vec![Vec::new(), f0];
    // Recycle the two trial tables across steps. The accepted table is swapped
    // with the spare after output, so history updates do not copy old columns.
    let mut trial = Vec::new();
    let mut next_phi = Vec::new();
    let mut psi = [0.; 14];
    let mut values = s.valt.unwrap_or_default();
    let mut next_output = 1;
    let mut order = 1;
    let mut age = 0;
    let initial_weights: Vec<f64> = weights(o, &y).iter().map(|w| 1. / w).collect();
    let rate = wnorm(o, &phi[1], &initial_weights);
    let limit = o.hmax.min(o.htspan).min((tf - t).abs());
    let mut h_abs = o
        .htry
        .unwrap_or_else(|| {
            if rate > 0. {
                limit.min(0.5 * libm::sqrt(o.rtol) / rate)
            } else {
                limit
            }
        })
        .min(limit);
    out.push(t, y.clone())?;
    let mut failures = 0;
    while direction * (tf - t) > 0. {
        m.check_cancel()?;
        let minimum = 16. * eps_of(t);
        let remaining = (tf - t).abs();
        // Clamp every proposal, including InitialStep and rejected retries.
        // A representable final remainder may be smaller than the usual floor.
        h_abs = h_abs.min(o.hmax).max(minimum).min(remaining);
        // HNW I, II.4: reach the endpoint when it is within 10% of this step,
        // avoiding an otherwise unnecessary tiny final interval.
        if remaining <= 1.1 * h_abs {
            h_abs = remaining;
        }
        let tnew = if h_abs == remaining {
            tf
        } else {
            t + direction * h_abs
        };
        let h = tnew - t;
        let degree = (phi.len() - 1).min(12).min(order + 2);
        let predictor_weights = adams113_integrals(h, order - 1, &psi);
        let predicted = adams113_sum(&y, &phi, &predictor_weights[..order]);
        let fp = rhs(m, tnew, &predicted)?;
        out.stats[2] += 1;
        adams113_update_into(fp, &phi, &psi, h, degree, &mut trial);
        let correction_weight = adams113_corrector(h, order, &psi);
        let correction: Vec<f64> = trial[order + 1]
            .iter()
            .map(|v| correction_weight * v)
            .collect();
        let mut ynew: Vec<f64> = predicted
            .iter()
            .zip(&correction)
            .map(|(a, b)| a + b)
            .collect();
        let old_weight = weights(o, &y);
        let inv_weight: Vec<f64> = weights(o, &ynew)
            .iter()
            .zip(old_weight)
            .map(|(a, b)| 1. / a.max(b))
            .collect();
        let error = if ynew.iter().all(|v| v.is_finite()) {
            wnorm(o, &correction, &inv_weight) / o.rtol
        } else {
            f64::INFINITY
        };
        if !error.is_finite() || error > 1. {
            out.stats[1] += 1;
            failures += 1;
            // A small proposal is not a failure: first try the minimum step.
            // Stop only when that trial (or a shorter final step) is rejected.
            if h_abs <= minimum {
                out.failure = Some((t, minimum));
                out.warnings.push(failure_warning("ode113", t, minimum));
                break;
            }
            h_abs = h.abs() * adams113_factor(error, order).clamp(0.1, 0.5);
            if failures >= 2 && order > 1 {
                order -= 1;
            }
            age = 0;
            continue;
        }

        let mut chosen_order = order;
        let mut growth = adams113_factor(error, order);
        // Compare estimated work at adjacent orders, changing only after enough
        // accepted history at the present order. The next-order defect uses
        // one additional divided difference, without another RHS evaluation.
        if age >= order {
            for candidate in [order.saturating_sub(1), order + 1] {
                if !(1..=12).contains(&candidate) || candidate > degree {
                    continue;
                }
                let coefficient = adams113_corrector(h, candidate, &psi);
                let defect: Vec<f64> = trial[candidate + 1]
                    .iter()
                    .map(|v| coefficient * v)
                    .collect();
                let estimate = wnorm(o, &defect, &inv_weight) / o.rtol;
                let candidate_growth = adams113_factor(estimate, candidate);
                if candidate_growth > growth * 1.2 {
                    chosen_order = candidate;
                    growth = candidate_growth;
                }
            }
        }
        let fnew = rhs(m, tnew, &ynew)?;
        out.stats[2] += 1;
        adams113_update_into(fnew, &phi, &psi, h, degree, &mut next_phi);
        let mut next_psi = [0.; 14];
        for j in 1..=12 {
            next_psi[j] = h + psi[j - 1];
        }
        out.stats[0] += 1;
        let mut endpoint = tnew;
        let mut stop = false;
        if o.events {
            let mut interpolate = |time| ntrp113(time, tnew, &ynew, order, &trial, &next_psi);
            let (found, right) = odezero(
                &mut interpolate,
                &mut |time, state| m.events(time, state),
                t0,
                &values,
                t,
                tnew,
                &ynew,
            )?;
            if (out.te.len() + found.t.len()).saturating_mul(n.saturating_add(2)) > MAX_VALUES {
                return Err(format!(
                    "Event output exceeds the {MAX_VALUES}-element limit."
                ));
            }
            stop = found.stop;
            if stop {
                endpoint = *found.t.last().unwrap();
                ynew = found.y.last().unwrap().clone();
                rebase113(endpoint - tnew, order, &mut trial, &mut next_psi);
            }
            out.add_events(&found);
            values = right;
        }
        let mut interpolate = |time| ntrp113(time, endpoint, &ynew, order, &trial, &next_psi);
        let data = if s.at == OutAt::Sol && !out.data_dropped {
            let mut saved = trial.clone();
            saved.resize_with(15, || vec![0.; n]);
            Some(StepData::Abm {
                k: order,
                phi: saved,
                psi: next_psi.to_vec(),
            })
        } else {
            None
        };
        out.step_output(
            s.at,
            s.tspan,
            &mut next_output,
            direction,
            t,
            endpoint,
            &ynew,
            stop,
            data,
            &mut interpolate,
        )?;
        if stop {
            break;
        }
        t = tnew;
        y = ynew;
        std::mem::swap(&mut phi, &mut next_phi);
        psi = next_psi;
        age = if chosen_order == order { age + 1 } else { 0 };
        order = chosen_order;
        h_abs = h.abs() * growth.clamp(0.5, if failures == 0 { 2. } else { 1. });
        failures = 0;
    }
    Ok(out)
}

/// Integral of the new-node Lagrange basis, evaluated on [0,1]. Multiplying
/// this by f(predicted) minus the extrapolated derivative is the AM correction.
fn adams113_corrector(h: f64, order: usize, psi: &[f64]) -> f64 {
    let mut polynomial = [0.; 13];
    polynomial[0] = 1.;
    for j in 1..=order {
        let denominator = h + psi[j - 1];
        let a = psi[j - 1] / denominator;
        let b = h / denominator;
        for r in (1..=j).rev() {
            polynomial[r] = a * polynomial[r] + b * polynomial[r - 1];
        }
        polynomial[0] *= a;
    }
    h * polynomial[..=order]
        .iter()
        .enumerate()
        .map(|(r, c)| c / (r + 1) as f64)
        .sum::<f64>()
}

fn adams113_factor(error: f64, order: usize) -> f64 {
    if !error.is_finite() {
        0.1
    } else if error <= 0. {
        8.
    } else {
        (0.85 * libm::pow(error, -1. / (order + 1) as f64)).min(8.)
    }
}

#[cfg(test)]
mod ode113_cleanroom {
    use super::*;

    struct Problem {
        kind: usize,
        calls: usize,
        mass_calls: usize,
        cancel: bool,
        event: Option<(f64, bool)>,
    }
    impl Model for Problem {
        fn f(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>> {
            self.calls += 1;
            Ok(match self.kind {
                1 => vec![y[1], -y[0]],
                2 => vec![2. * t],
                3 => y.iter().map(|_| 0.).collect(),
                4 => y.iter().map(|v| -20. * v).collect(),
                5 => y.iter().map(|v| v * v).collect(),
                6 => y.iter().map(|_| 1.).collect(),
                7 => y.iter().map(|_| if t == 1. { 0. } else { 1e20 }).collect(),
                _ => y.iter().map(|v| -v).collect(),
            })
        }
        fn jac(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            unreachable!()
        }
        fn mass(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            self.mass_calls += 1;
            Ok(vec![2.])
        }
        fn events(&mut self, t: f64, y: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
            let (root, state) = self.event.unwrap();
            Ok((
                vec![if state { y[0] - root } else { t - root }],
                vec![2.],
                vec![0.],
            ))
        }
        fn check_cancel(&self) -> R<()> {
            if self.cancel {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        }
    }
    fn options(tolerance: f64) -> Opts {
        Opts {
            rtol: tolerance,
            atol: vec![tolerance * 0.01; 2],
            normcontrol: false,
            threshold: vec![0.01; 2],
            hmax: 0.3,
            htry: None,
            htspan: 1.,
            maxorder: 5,
            bdf: false,
            jac: JacKind::Numeric,
            jconstant: false,
            mass: MassKind::None,
            mass_singular: "no".into(),
            initial_slope: None,
            events: false,
        }
    }
    fn problem(kind: usize) -> Problem {
        Problem {
            kind,
            calls: 0,
            mass_calls: 0,
            cancel: false,
            event: None,
        }
    }
    fn integrate(p: &mut Problem, times: &[f64], y: &[f64], o: &Opts, at: OutAt) -> R<Out> {
        let f0 = p.f(times[0], y)?;
        let valt = if o.events {
            Some(p.events(times[0], y)?.0)
        } else {
            None
        };
        ode113(
            p,
            Setup {
                tspan: times,
                y0: y,
                f0,
                valt,
                m0: matches!(o.mass, MassKind::Function { .. }).then(|| vec![2.]),
                at,
            },
            o,
        )
    }

    #[test]
    fn newton_integrals_and_updates_reproduce_nonuniform_polynomials() {
        // f(t)=1+2t+3t^2 on deliberately unequal intervals, both directions.
        for dir in [-1., 1.] {
            let f = |t: f64| 1. + 2. * t + 3. * t * t;
            let mut phi = vec![Vec::new(), vec![f(0.)]];
            let mut psi = [0.; 14];
            let mut t = 0.;
            for (degree, step) in [0.15, 0.3, 0.08, 0.2].into_iter().enumerate() {
                let h = dir * step;
                phi = adams113_update(vec![f(t + h)], &phi, &psi, h, (degree + 1).min(3));
                for j in (1..=12).rev() {
                    psi[j] = h + psi[j - 1];
                }
                t += h;
            }
            let primitive = |x: f64| x + x * x + x * x * x;
            for fraction in [0., 0.2, 0.5, 1.] {
                let x = t - dir * 0.2 * fraction;
                let actual = ntrp113(x, t, &[primitive(t)], 3, &phi, &psi)[0];
                assert!(
                    (actual - primitive(x)).abs() < 2e-14,
                    "{actual} != {}",
                    primitive(x)
                );
            }
        }
        let psi: Vec<f64> = (0..14).map(|j| j as f64).collect();
        assert_eq!(adams113_corrector(1., 1, &psi), 0.5);
        assert!((adams113_corrector(1., 2, &psi) - 5. / 12.).abs() < 1e-15);
    }

    #[test]
    fn decay_requested_times_and_saved_dense_output_work_both_directions() {
        for dir in [-1., 1.] {
            for tolerance in [1e-3, 1e-8, 1e-11] {
                let mut p = problem(0);
                let mut o = options(tolerance);
                o.hmax = 0.1;
                let times = [0., dir * 0.5, dir];
                let out = integrate(&mut p, &times, &[1.], &o, OutAt::Requested).unwrap();
                assert_eq!(out.t, times);
                assert!(out.failure.is_none());
                assert_eq!(out.stats[2], p.calls);
                assert_eq!(out.stats[2], 1 + 2 * out.stats[0] + out.stats[1]);
                for (t, y) in out.t.iter().zip(&out.y) {
                    assert!(
                        (y[0] - libm::exp(-t)).abs() < 10. * tolerance,
                        "tol={tolerance} t={t} y={y:?}"
                    );
                }
                let saved = integrate(&mut problem(0), &[0., dir], &[1.], &o, OutAt::Sol).unwrap();
                for (j, data) in saved.data.iter().enumerate() {
                    let StepData::Abm { k, phi, psi } = data else {
                        unreachable!()
                    };
                    assert!((1..=12).contains(k));
                    assert_eq!(phi.len(), 15);
                    let x = (saved.t[j] + saved.t[j + 1]) * 0.5;
                    let v = ntrp113(x, saved.t[j + 1], &saved.y[j + 1], *k, phi, psi)[0];
                    assert!((v - libm::exp(-x)).abs() < 10. * tolerance);
                    let left =
                        ntrp113(saved.t[j], saved.t[j + 1], &saved.y[j + 1], *k, phi, psi)[0];
                    assert!((left - saved.y[j][0]).abs() < 2e-13);
                }
            }
        }
    }

    #[test]
    fn oscillator_tight_tolerance_and_norm_control() {
        for normcontrol in [false, true] {
            let mut o = options(1e-10);
            o.normcontrol = normcontrol;
            let out = integrate(
                &mut problem(1),
                &[0., std::f64::consts::PI],
                &[0., 1.],
                &o,
                OutAt::Sol,
            )
            .unwrap();
            let y = out.y.last().unwrap();
            assert!(
                y[0].abs() < 1e-8 && (y[1] + 1.).abs() < 1e-8,
                "{y:?} {:?}",
                out.stats
            );
            assert!(out.stats[0] < 500);
            assert!(
                out.data
                    .iter()
                    .any(|d| matches!(d, StepData::Abm { k, .. } if *k >= 5))
            );
        }
    }

    #[test]
    fn component_blocks_preserve_one_worker_results_including_the_tail() {
        let run = || {
            let n = 131075;
            let mut o = options(1e-8);
            o.atol = vec![1e-10; n];
            o.threshold = vec![0.01; n];
            integrate(
                &mut problem(0),
                &[0., 0.15, 0.3],
                &vec![1.; n],
                &o,
                OutAt::Requested,
            )
            .unwrap()
        };
        let serial = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(run);
        let parallel = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(run);
        assert_eq!(serial.t, parallel.t);
        assert_eq!(serial.y, parallel.y);
        assert_eq!(serial.stats, parallel.stats);
        assert!(
            serial
                .y
                .last()
                .unwrap()
                .iter()
                .all(|y| (y - libm::exp(-0.3)).abs() < 1e-8)
        );
    }

    #[test]
    fn parallel_difference_and_interpolation_kernels_match_scalar_arithmetic() {
        let run = || {
            // Force both measured thresholds, including a partial last block.
            let n = ADAMS113_PARALLEL_WORK / 12 + 3;
            let phi: Vec<Vec<f64>> = (0..14)
                .map(|j| {
                    (0..n)
                        .map(|i| (1. + (i % 17) as f64) * 0.01f64.powi(j))
                        .collect()
                })
                .collect();
            let psi: Vec<f64> = (0..14).map(|j| 0.1 * j as f64).collect();
            let updated = adams113_update(vec![1.; n], &phi, &psi, 0.07, 12);
            let coefficients = adams113_integrals(-0.05, 12, &psi);
            let value = adams113_sum(&vec![2.; n], &updated, &coefficients);
            (updated, value)
        };
        let serial = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(run);
        let parallel = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(run);
        assert_eq!(serial, parallel);
    }

    #[test]
    fn rejection_and_mass_solve_preserve_accuracy_and_accounting() {
        let mut o = options(1e-8);
        o.htry = Some(0.3);
        let mut p = problem(4);
        let out = integrate(&mut p, &[0., 0.5], &[1.], &o, OutAt::Steps).unwrap();
        assert!(out.stats[1] > 0);
        assert!((out.y.last().unwrap()[0] - libm::exp(-10.)).abs() < 1e-8);
        assert_eq!(p.calls, out.stats[2]);
        o.mass = MassKind::Matrix(vec![2.]);
        let constant = integrate(&mut problem(0), &[0., 1.], &[1.], &o, OutAt::Sol).unwrap();
        o.mass = MassKind::Function { state: true };
        let mut p = problem(0);
        let function = integrate(&mut p, &[0., 1.], &[1.], &o, OutAt::Sol).unwrap();
        assert_eq!(constant.y, function.y);
        assert_eq!(p.mass_calls, p.calls - 1);
        assert!((constant.y.last().unwrap()[0] - libm::exp(-0.5)).abs() < 1e-8);
    }

    #[test]
    fn terminal_event_rebases_the_accepted_polynomial() {
        for direction in [-1., 1.] {
            let mut o = options(1e-8);
            o.htry = Some(1.);
            o.hmax = 1.;
            let full = integrate(&mut problem(2), &[0., direction], &[0.], &o, OutAt::Sol).unwrap();
            o.events = true;
            let mut p = problem(2);
            p.event = Some((direction * 0.5, false));
            let cut = integrate(&mut p, &[0., direction], &[0.], &o, OutAt::Sol).unwrap();
            assert!((cut.t.last().unwrap() - direction * 0.5).abs() < 1e-12);
            assert_eq!(cut.ie, [1]);
            for x in [direction * 0.25, direction * 0.49] {
                let eval = |out: &Out| {
                    let b = out.t.partition_point(|t| direction * (*t - x) < 0.).max(1);
                    let StepData::Abm { k, phi, psi } = &out.data[b - 1] else {
                        unreachable!()
                    };
                    ntrp113(x, out.t[b], &out.y[b], *k, phi, psi)[0]
                };
                assert!((eval(&cut) - x * x).abs() < 1e-8);
                assert!((eval(&cut) - eval(&full)).abs() < 1e-12);
            }
            let times = [
                0.,
                direction * 0.25,
                direction * 0.5,
                direction * 0.75,
                direction,
            ];
            let requested = integrate(&mut p, &times, &[0.], &o, OutAt::Requested).unwrap();
            assert_eq!(requested.t, [times[0], times[1], times[2], times[2]]);
            assert_eq!(requested.y.len(), 4);
            assert_eq!(requested.y[2], requested.y[3]);
        }
    }

    #[test]
    fn nonlinear_growth_and_shifted_time_origin_match_closed_forms() {
        let o = options(1e-9);
        let out = integrate(
            &mut problem(5),
            &[0., 0.5, 0.9],
            &[1.],
            &o,
            OutAt::Requested,
        )
        .unwrap();
        assert!(out.failure.is_none());
        for (t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - 1. / (1. - t)).abs() < 1e-7, "{t}: {y:?}");
        }
        let out = integrate(
            &mut problem(0),
            &[1e6, 1e6 + 0.5, 1e6 + 1.],
            &[1.],
            &o,
            OutAt::Requested,
        )
        .unwrap();
        assert!(out.failure.is_none());
        for (t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - libm::exp(1e6 - t)).abs() < 1e-8);
        }
    }

    #[test]
    fn subminimum_initial_steps_are_clamped_and_integrate_normally() {
        for direction in [-1., 1.] {
            for (start, initial) in [(1e6, None), (1., Some(1e-20))] {
                let mut o = options(1e-6);
                o.atol = vec![1e-12];
                o.threshold = vec![1e-6];
                o.htry = initial;
                let out = integrate(
                    &mut problem(6),
                    &[start, start + direction],
                    &[0.],
                    &o,
                    OutAt::Steps,
                )
                .unwrap();
                assert!(out.failure.is_none(), "start={start} {:?}", out.failure);
                assert_eq!(*out.t.last().unwrap(), start + direction);
                assert!(out.stats[0] > 0);
                assert_eq!(out.stats[1], 0);
                assert!((out.y.last().unwrap()[0] - direction).abs() < 1e-12);
                assert!((out.t[1] - start).abs() >= 16. * eps_of(start));
            }
        }
    }

    #[test]
    fn final_step_stretches_to_avoid_a_tiny_remainder() {
        for direction in [-1., 1.] {
            let mut o = options(1e-6);
            o.htry = Some(0.1);
            o.hmax = 0.1;
            let out =
                integrate(&mut problem(6), &[0., direction], &[0.], &o, OutAt::Steps).unwrap();
            assert!(out.failure.is_none());
            assert_eq!(out.t.len(), 11, "{:?}", out.t);
            assert_eq!(*out.t.last().unwrap(), direction);
            assert!((out.t[9] - direction * 0.9).abs() < 2e-16);
            assert!((out.y.last().unwrap()[0] - direction).abs() < 1e-14);
        }
        // Exercise the full 10% window, rather than only last-bit roundoff.
        let mut o = options(1e-6);
        o.htry = Some(0.1);
        o.hmax = 0.1;
        let out = integrate(&mut problem(6), &[0., 0.105], &[0.], &o, OutAt::Steps).unwrap();
        assert_eq!(out.t, [0., 0.105]);
    }

    #[test]
    fn cancellation_and_minimum_step_failure_are_observable() {
        let mut p = problem(0);
        p.cancel = true;
        assert!(
            integrate(&mut p, &[0., 1.], &[1.], &options(1e-8), OutAt::Steps)
                .err()
                .unwrap()
                .contains("cancelled")
        );
        for initial in [1e-30, 1e-10] {
            let mut o = options(1e-8);
            o.htry = Some(initial);
            let mut p = problem(7);
            let out = integrate(&mut p, &[1., 2.], &[1.], &o, OutAt::Steps).unwrap();
            assert_eq!(out.t, [1.]);
            assert_eq!(out.stats[0], 0);
            assert_eq!(out.stats[2], 1 + out.stats[1]);
            assert_eq!(p.calls, out.stats[2]);
            if initial < 16. * eps_of(1.) {
                assert_eq!(out.stats[1], 1);
            } else {
                assert!(out.stats[1] > 1);
            }
            assert_eq!(out.failure, Some((1., 16. * eps_of(1.))));
            assert!(out.warnings[0].starts_with("Warning: Integration stopped at t="));
        }
    }
}

/// Jacobian state shared by the implicit solvers.
struct JacState {
    constant: bool,
    analytic: bool,
    thresh: Vec<f64>,
    fac: Vec<f64>,
}

impl JacState {
    fn refresh(
        &mut self,
        m: &mut dyn Model,
        f: &mut FCall,
        t: f64,
        y: &[f64],
        stats: &mut [usize; 6],
    ) -> R<Vec<f64>> {
        if self.analytic {
            let result = m.jac(t, y)?;
            stats[3] += 1;
            Ok(result)
        } else {
            m.check_cancel()?;
            let value = f(m, t, y)?;
            stats[2] += 1;
            self.eval(m, f, t, y, &value, stats)
        }
    }

    fn new(o: &Opts) -> JacState {
        JacState {
            constant: o.jconstant || matches!(o.jac, JacKind::Matrix(_)),
            analytic: matches!(o.jac, JacKind::Function),
            thresh: o.atol.clone(),
            fac: Vec::new(),
        }
    }
    fn eval(
        &mut self,
        m: &mut dyn Model,
        f: &mut FCall,
        t: f64,
        y: &[f64],
        fty: &[f64],
        stats: &mut [usize; 6],
    ) -> R<Vec<f64>> {
        let result = if self.analytic {
            m.jac(t, y)?
        } else {
            let (matrix, calls) = numjac(
                &mut |state| {
                    m.check_cancel()?;
                    f(m, t, state)
                },
                y,
                fty,
                &self.thresh,
                &mut self.fac,
            )?;
            stats[2] += calls;
            matrix
        };
        stats[3] += 1;
        Ok(result)
    }
}

/// Miter = M - c * dfdy (column-major).
fn iteration_matrix(mt: &[f64], c: f64, dfdy: &[f64]) -> Vec<f64> {
    if mt.is_empty() {
        let n = (dfdy.len() as f64).sqrt() as usize;
        dfdy.iter()
            .enumerate()
            .map(|(i, d)| f64::from(i % (n + 1) == 0) - c * d)
            .collect()
    } else {
        mt.iter().zip(dfdy).map(|(m, d)| m - c * d).collect()
    }
}

/// Row scaling of DAE iteration matrices: the reciprocal of each row's
/// largest magnitude.
fn row_scale(miter: &[f64], n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let mut big = 0f64;
            for j in 0..n {
                big = mmax(big, miter[j * n + i].abs());
            }
            1. / big
        })
        .collect()
}

fn scale_rows(miter: &mut [f64], n: usize, rs: &[f64]) {
    for j in 0..n {
        for i in 0..n {
            miter[j * n + i] *= rs[i];
        }
    }
}

/// 1-norm condition number of a square matrix (Inf when singular), standing in
/// for condest in the MassSingular 'maybe' test.
fn cond1(a: &[f64], n: usize) -> f64 {
    let norm1 = |m: &[f64]| -> f64 {
        (0..n)
            .map(|j| m[j * n..(j + 1) * n].iter().map(|v| v.abs()).sum::<f64>())
            .fold(0., mmax)
    };
    let lu = Lu::new(a, n);
    let mut inv = vec![0.; n * n];
    for j in 0..n {
        let mut e = vec![0.; n];
        e[j] = 1.;
        let col = lu.solve(&e);
        if col.iter().any(|v| !v.is_finite()) {
            return f64::INFINITY;
        }
        inv[j * n..(j + 1) * n].copy_from_slice(&col);
    }
    norm1(a) * norm1(&inv)
}

/// Whether a mass matrix is singular: MassSingular 'yes', 'no' or 'maybe'.
fn is_dae(o: &Opts, mt: &[f64], n: usize) -> bool {
    match o.mass_singular.as_str() {
        "yes" => true,
        "no" => false,
        _ => {
            let nz = mt.iter().filter(|v| **v != 0.).count() as f64;
            f64::EPSILON * nz * cond1(mt, n) > 1.
        }
    }
}

/// Consistent initial conditions of a DAE with a diagonal mass matrix
/// (semi-explicit index-1 form): the algebraic components solve g(t, y) = 0
/// by Newton's method with the differential ones held fixed; then
/// yp_d = f_d / M_dd and g_a yp_a = -(g_t + g_d yp_d).
/// Shampine, Reichelt & Kierzenka (1999), sections 3–4.
#[allow(clippy::too_many_arguments)]
fn daeic_diagonal(
    m: &mut dyn Model,
    f: &mut FCall,
    jac: &mut JacState,
    jconst: Option<&Vec<f64>>,
    t: f64,
    mt: &[f64],
    y: &mut [f64],
    f0: &mut Vec<f64>,
    rtol: f64,
    threshold: &[f64],
    stats: &mut [usize; 6],
) -> R<(Vec<f64>, Vec<f64>)> {
    // Preserve the interface and differencing policy of the other callers.
    daeic_diagonal_start(
        m, f, jac, jconst, t, None, mt, y, f0, rtol, threshold, stats,
    )
}

#[allow(clippy::too_many_arguments)]
fn daeic_diagonal_start(
    m: &mut dyn Model,
    f: &mut FCall,
    jac: &mut JacState,
    jconst: Option<&Vec<f64>>,
    t: f64,
    tfinal: Option<f64>,
    mt: &[f64],
    y: &mut [f64],
    f0: &mut Vec<f64>,
    rtol: f64,
    threshold: &[f64],
    stats: &mut [usize; 6],
) -> R<(Vec<f64>, Vec<f64>)> {
    let n = y.len();
    if (0..n).any(|j| (0..n).any(|i| i != j && mt[j * n + i] != 0.)) {
        return Err(
            "Consistent initial conditions require a constant diagonal mass matrix.".into(),
        );
    }
    let algebraic: Vec<_> = (0..n).filter(|&i| mt[i * n + i] == 0.).collect();
    let na = algebraic.len();
    let mut matrix = match jconst {
        Some(j) => j.clone(),
        None => jac.eval(m, f, t, y, f0, stats)?,
    };
    let mut converged = na == 0;
    for iteration in 0..15 {
        if converged {
            break;
        }
        m.check_cancel()?;
        let block: Vec<_> = algebraic
            .iter()
            .flat_map(|&j| algebraic.iter().map(move |&i| (i, j)))
            .map(|(i, j)| matrix[j * n + i])
            .collect();
        let lu = Lu::new(&block, na);
        stats[4] += 1;
        let rhs: Vec<_> = algebraic.iter().map(|&i| -f0[i]).collect();
        let change = lu.solve(&rhs);
        stats[5] += 1;
        if change.iter().any(|x| !x.is_finite()) {
            break;
        }
        let size = norm_inf(
            algebraic
                .iter()
                .zip(&change)
                .map(|(&i, &d)| d / y[i].abs().max(threshold[i]).max(f64::MIN_POSITIVE)),
        );
        for (&i, &d) in algebraic.iter().zip(&change) {
            y[i] += d;
        }
        *f0 = f(m, t, y)?;
        stats[2] += 1;
        // A small correction alone cannot accept a nonfinite residual.
        converged =
            size <= (0.01 * rtol).max(100. * f64::EPSILON) && f0.iter().all(|v| v.is_finite());
        if !jac.constant && jconst.is_none() && (iteration > 0 || !converged) {
            matrix = jac.eval(m, f, t, y, f0, stats)?;
        }
    }
    if !converged {
        return Err("Unable to find consistent initial conditions for the DAE.".into());
    }
    let mut slope = vec![0.; n];
    for i in 0..n {
        if mt[i * n + i] != 0. {
            slope[i] = f0[i] / mt[i * n + i];
        }
    }
    if na != 0 {
        // Differentiate g(t,y)=0. Omitting g_t gives a wrong predictor for
        // time-dependent algebraic constraints even when y0 is consistent.
        let time_derivative = if let Some(tfinal) = tfinal {
            // Sample toward tfinal: the model need only be defined on tspan.
            // Use the actual representable increment in the forward quotient.
            let remaining = (tfinal - t).abs();
            let dt = (f64::EPSILON.sqrt() * t.abs().max(1.)).min(remaining);
            let probe = if dt == remaining {
                tfinal
            } else {
                t + sign(tfinal - t) * dt
            };
            let fp = f(m, probe, y)?;
            stats[2] += 1;
            fp.iter()
                .zip(f0.iter())
                .map(|(a, b)| (a - b) / (probe - t))
                .collect::<Vec<_>>()
        } else {
            let dt = f64::EPSILON.cbrt() * t.abs().max(1.);
            let fp = f(m, t + dt, y)?;
            let fm = f(m, t - dt, y)?;
            stats[2] += 2;
            fp.iter()
                .zip(&fm)
                .map(|(a, b)| (a - b) / (2. * dt))
                .collect()
        };
        let block: Vec<_> = algebraic
            .iter()
            .flat_map(|&j| algebraic.iter().map(move |&i| (i, j)))
            .map(|(i, j)| matrix[j * n + i])
            .collect();
        let jacobian_slope = matvec(&matrix, n, &slope);
        let rhs: Vec<_> = algebraic
            .iter()
            .map(|&i| -time_derivative[i] - jacobian_slope[i])
            .collect();
        let derivative = Lu::new(&block, na).solve(&rhs);
        stats[4] += 1;
        stats[5] += 1;
        for (&i, &v) in algebraic.iter().zip(&derivative) {
            slope[i] = v;
        }
    }
    if slope.iter().any(|v| !v.is_finite()) {
        return Err("Unable to find consistent initial conditions for the DAE.".into());
    }
    Ok((slope, matrix))
}

/// Re-express the same Newton polynomial on a new equally spaced mesh.
/// Shampine & Reichelt (1997), section 2.2. Applying backward differences to
/// each basis polynomial separately avoids subtracting nearly equal states.
fn ndf_rebase(dif: &mut [Vec<f64>], k: usize, maxk: usize, shift: f64, ratio: f64) {
    let n = dif[1].len();
    let mut next = vec![vec![0.; n]; k + 1];
    for (j, column) in dif.iter().enumerate().take(k + 1).skip(1) {
        let mut samples = [0.; 6];
        for (l, sample) in samples.iter_mut().enumerate().take(k + 1) {
            let x = shift - l as f64 * ratio;
            let mut basis = 1.;
            for degree in 1..=j {
                basis *= (x + (degree - 1) as f64) / degree as f64;
            }
            *sample = basis;
        }
        for (degree, output) in next.iter_mut().enumerate().take(k + 1).skip(1) {
            for l in 0..=k - degree {
                samples[l] -= samples[l + 1];
            }
            for (v, old) in output.iter_mut().zip(column) {
                *v += samples[0] * old;
            }
        }
    }
    for j in 1..=k {
        dif[j].copy_from_slice(&next[j]);
    }
    for column in dif.iter_mut().take(maxk + 3).skip(k + 1) {
        column.fill(0.);
    }
}

fn dif_rescale(dif: &mut [Vec<f64>], k: usize, maxk: usize, ratio: f64) {
    ndf_rebase(dif, k, maxk, 0., ratio);
}

/// Newton backward interpolation, Shampine & Reichelt (1997), section 2.
pub fn ntrp15s(
    tinterp: f64,
    tnew: f64,
    ynew: &[f64],
    h: f64,
    dif: &[Vec<f64>],
    k: usize,
) -> Vec<f64> {
    let x = (tinterp - tnew) / h;
    let mut result = ynew.to_vec();
    let mut basis = 1.;
    for (j, difference) in dif.iter().enumerate().take(k + 1).skip(1) {
        basis *= (x + (j - 1) as f64) / j as f64;
        for (value, d) in result.iter_mut().zip(difference) {
            *value += basis * d;
        }
    }
    result
}

/// The mass-matrix-aware derivative used by the implicit solvers: f itself.
fn plain_f(m: &mut dyn Model, t: f64, y: &[f64]) -> R<Vec<f64>> {
    m.f(t, y)
}

/// Variable-order NDF/BDF in backward differences. Shampine & Reichelt,
/// The MATLAB ODE Suite (1997), section 2, with mass-matrix residuals as in
/// Shampine, Reichelt & Kierzenka (1999), sections 3–4. The chord iteration
/// solves linear systems; no inverse of the Jacobian or mass is formed.
pub fn ode15s(m: &mut dyn Model, s: Setup, o: &Opts) -> R<Out> {
    m.check_cancel()?;
    let n = s.y0.len();
    let maxk = o.maxorder.clamp(1, 5);
    let t0 = s.tspan[0];
    let end = *s.tspan.last().unwrap();
    let direction = sign(end - t0);
    let mut t = t0;
    let mut y = s.y0.to_vec();
    let mut fvalue = s.f0;
    let mut out = Out::new();
    out.stats[2] = 1; // The setup evaluated f(t0,y0).
    let mut mass = MassState::new(o, n, s.m0);
    if mass.mtype != 0 && mass.mt.iter().all(|v| *v == 0.) {
        return Err(
            "If the mass matrix is all zero, the problem is not a differential algebraic equation."
                .into(),
        );
    }
    let dae = mass.mtype != 0 && is_dae(o, &mass.mt, n);
    let mut jac = JacState::new(o);
    let jconst = match &o.jac {
        JacKind::Matrix(j) => Some(j),
        _ => None,
    };
    let (slope, mut dfdy) = if dae {
        // A supplied consistent pair needs no initialization solve, including
        // for non-diagonal or functional singular mass matrices. Check the
        // complete residual M*yp-f, including its algebraic equations.
        let supplied = o.initial_slope.as_ref().filter(|initial| {
            if initial.len() != n || initial.iter().any(|v| !v.is_finite()) {
                return false;
            }
            let product = mass.times(initial);
            product.iter().zip(&fvalue).enumerate().all(|(i, (a, b))| {
                let atol = o.atol[if o.atol.len() == 1 { 0 } else { i }];
                a.is_finite()
                    && b.is_finite()
                    && (a - b).abs() <= atol + o.rtol * a.abs().max(b.abs())
            })
        });
        if let Some(initial) = supplied {
            let matrix = match jconst {
                Some(j) => j.clone(),
                None => jac.eval(m, &mut plain_f, t, &y, &fvalue, &mut out.stats)?,
            };
            (initial.clone(), matrix)
        } else {
            if mass.mtype != 1 {
                return Err(
                    "Consistent initial conditions require a constant diagonal mass matrix.".into(),
                );
            }
            daeic_diagonal_start(
                m,
                &mut plain_f,
                &mut jac,
                jconst,
                t,
                Some(end),
                &mass.mt,
                &mut y,
                &mut fvalue,
                o.rtol,
                &o.threshold,
                &mut out.stats,
            )?
        }
    } else {
        let derivative = if mass.mtype == 0 {
            fvalue.clone()
        } else {
            out.stats[4] += 1;
            out.stats[5] += 1;
            Lu::new(&mass.mt, n).solve(&fvalue)
        };
        let matrix = match jconst {
            Some(j) => j.clone(),
            None => jac.eval(m, &mut plain_f, t, &y, &fvalue, &mut out.stats)?,
        };
        (derivative, matrix)
    };
    out.push(t, y.clone())?;
    let mut event_values = s.valt;
    if dae && o.events {
        let (v, term, dir) = m.events(t, &y)?;
        super::num::validate_events(&v, &term, &dir, event_values.as_ref().map_or(0, Vec::len))?;
        event_values = Some(v);
    }
    let inverse: Vec<_> = weights(o, &y).iter().map(|w| 1. / w).collect();
    let speed = wnorm(o, &slope, &inverse);
    let limit = o.hmax.min(f64::MAX).min((end - t).abs());
    let mut h = direction
        * o.htry
            .unwrap_or_else(|| {
                let proposed = 0.01 * limit.min(o.htspan);
                if speed > 0. && speed.is_finite() {
                    proposed.min(0.1 * o.rtol.sqrt() / speed)
                } else {
                    proposed
                }
            })
            .min(limit)
            .max(16. * eps_of(t));
    let mut dif = vec![vec![0.; n]; maxk + 3];
    for (d, v) in dif[1].iter_mut().zip(&slope) {
        *d = h * v;
    }
    // alpha multiplies the predictor correction in the discrete residual;
    // error multiplies that correction in the local truncation error estimate.
    let kappa = if o.bdf {
        [0.; 6]
    } else {
        [0., -0.185, -1. / 9., -0.0823, -0.0415, 0.]
    };
    let mut gamma = [0.; 6];
    let mut alpha = [0.; 6];
    let mut error = [0.; 6];
    for k in 1..=5 {
        gamma[k] = gamma[k - 1] + 1. / k as f64;
        alpha[k] = (1. - kappa[k]) * gamma[k];
        error[k] = kappa[k] * gamma[k] + 1. / (k + 1) as f64;
    }
    let mut order = 1;
    let mut equal_steps = 0;
    let mut next_output = 1;
    let mut factor: Option<Lu> = None;
    let mut factor_c = 0.;
    let mut rows = vec![1.; n];
    let newton_tolerance = (0.03 * o.rtol).max(10. * f64::EPSILON);
    let mut fresh_step = true;
    while direction * (end - t) > 0. {
        m.check_cancel()?;
        let minimum = 16. * eps_of(t);
        let remaining = (end - t).abs();
        let proposed = h.abs().min(o.hmax);
        // Shampine & Reichelt (1997), section 2: stretch a fresh step to
        // the endpoint when it is within ten percent. Never undo a retry's
        // reduction after a rejected trial.
        let step = if fresh_step && 1.1 * proposed >= remaining {
            remaining
        } else {
            proposed.min(remaining)
        };
        if step < minimum || t + direction * step == t {
            out.failure = Some((t, minimum));
            out.warnings.push(failure_warning("ode15s", t, minimum));
            break;
        }
        if step != h.abs() {
            dif_rescale(&mut dif, order, maxk, step / h.abs());
            equal_steps = 0;
            factor = None;
            h = direction * step;
        }
        let tnew = if step == remaining { end } else { t + h };
        let c = h / alpha[order];
        let mut prediction = y.clone();
        let mut psi = vec![0.; n];
        for j in 1..=order {
            for i in 0..n {
                prediction[i] += dif[j][i];
                psi[i] += gamma[j] / alpha[order] * dif[j][i];
            }
        }
        let mut trial = prediction.clone();
        let mut correction = vec![0.; n];
        let inverse: Vec<_> = if o.normcontrol {
            vec![1. / norm2(&prediction).max(norm2(&y)).max(o.atol[0] / o.rtol); n]
        } else {
            prediction
                .iter()
                .zip(&y)
                .enumerate()
                .map(|(i, (a, b))| {
                    let atol = o.atol[if o.atol.len() == 1 { 0 } else { i }];
                    1. / a.abs().max(b.abs()).max(atol / o.rtol)
                })
                .collect()
        };
        let mut converged = false;
        let mut refreshed = false;
        // One retry with a current Jacobian precedes a step reduction.
        loop {
            let mut previous_norm = f64::INFINITY;
            for iteration in 0..6 {
                m.check_cancel()?;
                if trial.iter().any(|v| !v.is_finite()) {
                    break;
                }
                if mass.mtype >= 2 && (iteration == 0 || mass.mtype == 3) {
                    mass.mt = mass.eval(m, tnew, &trial)?;
                    factor = None;
                }
                if factor.is_none() || (c - factor_c).abs() > 8. * f64::EPSILON * c.abs() {
                    let mut matrix = iteration_matrix(&mass.mt, c, &dfdy);
                    if dae {
                        rows = row_scale(&matrix, n);
                        scale_rows(&mut matrix, n, &rows);
                    }
                    factor = Some(Lu::new(&matrix, n));
                    factor_c = c;
                    out.stats[4] += 1;
                }
                fvalue = m.f(tnew, &trial)?;
                out.stats[2] += 1;
                if fvalue.iter().any(|v| !v.is_finite()) {
                    break;
                }
                let derivative: Vec<_> = psi.iter().zip(&correction).map(|(a, b)| a + b).collect();
                let residual: Vec<_> = fvalue
                    .iter()
                    .zip(mass.times(&derivative))
                    .zip(&rows)
                    .map(|((f, md), scale)| (c * f - md) * scale)
                    .collect();
                let change = factor.as_ref().unwrap().solve(&residual);
                out.stats[5] += 1;
                let magnitude = wnorm(o, &change, &inverse);
                if !magnitude.is_finite() {
                    break;
                }
                for i in 0..n {
                    correction[i] += change[i];
                    trial[i] = prediction[i] + correction[i];
                }
                if trial.iter().any(|v| !v.is_finite()) {
                    break;
                }
                let rate = magnitude / previous_norm;
                // A stale stiff Jacobian can map a large residual to a tiny
                // correction. Require an observed contraction before using
                // the tail estimate; a small first correction is not proof
                // of convergence. Only roundoff-sized corrections bypass it.
                if magnitude <= 10. * f64::EPSILON
                    || (iteration > 0
                        && rate < 0.9
                        && magnitude * rate / (1. - rate) <= newton_tolerance)
                {
                    converged = true;
                    break;
                }
                if iteration > 0 && rate >= 0.9 {
                    break;
                }
                previous_norm = magnitude;
            }
            if converged
                || refreshed
                || jac.constant
                || trial.iter().chain(&fvalue).any(|v| !v.is_finite())
            {
                break;
            }
            dfdy = jac.refresh(m, &mut plain_f, tnew, &trial, &mut out.stats)?;
            factor = None;
            refreshed = true;
            trial.copy_from_slice(&prediction);
            correction.fill(0.);
        }
        let estimate = if converged {
            error[order] * wnorm(o, &correction, &inverse) / o.rtol
        } else {
            f64::INFINITY
        };
        if !estimate.is_finite() || estimate > 1. {
            out.stats[1] += 1;
            let shrink = if converged && estimate.is_finite() {
                (0.8 * estimate.powf(-1. / (order + 1) as f64)).clamp(0.1, 0.5)
            } else {
                0.5
            };
            h *= shrink;
            dif_rescale(&mut dif, order, maxk, shrink);
            if equal_steps == 0 && order > 1 {
                order -= 1;
                dif[order + 1].fill(0.);
            }
            equal_steps = 0;
            factor = None;
            fresh_step = false;
            continue;
        }
        out.stats[0] += 1;
        equal_steps += 1;
        for (i, &d) in correction.iter().enumerate() {
            dif[order + 2][i] = d - dif[order + 1][i];
            dif[order + 1][i] = d;
        }
        for j in (1..=order).rev() {
            for i in 0..n {
                dif[j][i] += dif[j + 1][i];
            }
        }
        let mut finish_t = tnew;
        let mut finish_y = trial.clone();
        let mut stop = false;
        let mut interpolate = |time| ntrp15s(time, tnew, &trial, h, &dif, order);
        if let Some(values) = event_values.as_ref() {
            let (found, right) = odezero(
                &mut interpolate,
                &mut |time, state| m.events(time, state),
                t0,
                values,
                t,
                tnew,
                &trial,
            )?;
            stop = found.stop;
            if stop {
                finish_t = *found.t.last().unwrap();
                finish_y = found.y.last().unwrap().clone();
            }
            out.add_events(&found);
            event_values = Some(right);
        }
        let data = if s.at == OutAt::Sol {
            let mut saved = dif.clone();
            // deval uses the stored (possibly cut) mesh interval as its h.
            // Translate and scale the polynomial, retaining the same curve.
            let saved_h = finish_t - t;
            if finish_t != tnew || saved_h != h {
                ndf_rebase(&mut saved, order, maxk, (finish_t - tnew) / h, saved_h / h);
            }
            Some(StepData::Ndf {
                k: order,
                dif: saved,
            })
        } else {
            None
        };
        out.step_output(
            s.at,
            s.tspan,
            &mut next_output,
            direction,
            t,
            finish_t,
            &finish_y,
            stop,
            data,
            &mut interpolate,
        )?;
        t = finish_t;
        y = finish_y;
        fresh_step = true;
        if stop || t == end {
            break;
        }
        // Only compare orders once the history has enough equal steps. This
        // amortizes factorization and avoids extrapolating startup transients.
        if equal_steps > order {
            let growth = |err: f64, degree: usize| {
                if err == 0. {
                    5.
                } else {
                    (0.9 * err.powf(-1. / (degree + 1) as f64)).clamp(0.2, 5.)
                }
            };
            let mut selected = order;
            let mut ratio = growth(estimate, order);
            if order > 1 {
                let low = error[order - 1] * wnorm(o, &dif[order], &inverse) / o.rtol;
                let candidate = growth(low, order - 1);
                if candidate > ratio * 1.1 {
                    ratio = candidate;
                    selected = order - 1;
                }
            }
            if order < maxk {
                let high = error[order + 1] * wnorm(o, &dif[order + 2], &inverse) / o.rtol;
                let candidate = growth(high, order + 1);
                if candidate > ratio * 1.1 {
                    ratio = candidate;
                    selected = order + 1;
                }
            }
            ratio = ratio.min(o.hmax.min(f64::MAX) / h.abs());
            if selected != order || !(0.8..1.2).contains(&ratio) {
                order = selected;
                dif_rescale(&mut dif, order, maxk, ratio);
                h *= ratio;
                equal_steps = 0;
                factor = None;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod ndf_tests {
    use super::*;

    type Callback = fn(f64, &[f64]) -> Vec<f64>;
    struct Problem {
        rhs: Callback,
        matrix: Option<Callback>,
        mass: Option<Callback>,
        event: Option<Callback>,
        event_shift: f64,
        calls: usize,
        jac_calls: usize,
        cancel_after: usize,
    }
    impl Problem {
        fn new(rhs: Callback) -> Self {
            Self {
                rhs,
                matrix: None,
                mass: None,
                event: None,
                event_shift: 0.,
                calls: 0,
                jac_calls: 0,
                cancel_after: usize::MAX,
            }
        }
    }
    impl Model for Problem {
        fn f(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>> {
            self.calls += 1;
            Ok((self.rhs)(t, y))
        }
        fn jac(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>> {
            self.jac_calls += 1;
            Ok(self.matrix.unwrap()(t, y))
        }
        fn mass(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>> {
            Ok(self.mass.unwrap()(t, y))
        }
        fn events(&mut self, t: f64, y: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
            let v: Vec<_> = self.event.unwrap()(t, y)
                .iter()
                .map(|v| v - self.event_shift)
                .collect();
            let n = v.len();
            Ok((v, vec![1.; n], vec![0.; n]))
        }
        fn check_cancel(&self) -> R<()> {
            if self.calls >= self.cancel_after {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        }
    }
    fn options(n: usize, rtol: f64) -> Opts {
        Opts {
            rtol,
            atol: vec![rtol * 1e-3; n],
            normcontrol: false,
            threshold: vec![1e-3; n],
            hmax: 0.1,
            htry: None,
            htspan: 1.,
            maxorder: 5,
            bdf: false,
            jac: JacKind::Numeric,
            jconstant: false,
            mass: MassKind::None,
            mass_singular: "maybe".into(),
            initial_slope: None,
            events: false,
        }
    }
    fn run(p: &mut Problem, times: &[f64], y: &[f64], o: &Opts, at: OutAt) -> Out {
        let f0 = p.f(times[0], y).unwrap();
        let valt = if o.events {
            Some(p.events(times[0], y).unwrap().0)
        } else {
            None
        };
        let m0 = if p.mass.is_some() {
            Some(p.mass(times[0], y).unwrap())
        } else {
            None
        };
        ode15s(
            p,
            Setup {
                tspan: times,
                y0: y,
                f0,
                valt,
                m0,
                at,
            },
            o,
        )
        .unwrap()
    }
    fn decay(_: f64, y: &[f64]) -> Vec<f64> {
        y.iter().map(|v| -v).collect()
    }
    fn check_counts(p: &Problem, out: &Out) {
        assert_eq!(out.stats[2], p.calls);
        if p.matrix.is_some() {
            assert_eq!(out.stats[3], p.jac_calls);
        }
        assert_eq!(out.stats[0], out.t.len() - 1);
        assert!(out.failure.is_none(), "{:?}", out.warnings);
        assert!(out.stats[4] <= out.stats[5]);
    }

    #[test]
    fn ndf_fix1_final_step_reaches_requested_endpoint() {
        for times in [[0., 0.5, 1.], [1., 0.5, 0.]] {
            let mut p = Problem::new(|_, _| vec![1.]);
            let mut o = options(1, 1e-3);
            o.htry = Some(0.1);
            let out = run(&mut p, &times, &[0.], &o, OutAt::Requested);
            println!("endpoint: t={:?} warnings={:?}", out.t, out.warnings);
            assert_eq!(out.t, times);
            assert!(out.failure.is_none() && out.warnings.is_empty());
            assert!((out.y[2][0] - (times[2] - times[0])).abs() < 1e-12);
        }
    }

    #[test]
    fn ndf_fix1_zero_mass_is_rejected() {
        for function in [false, true] {
            let mut p = Problem::new(decay);
            p.mass = function.then_some(|_, _| vec![0.]);
            let mut o = options(1, 1e-3);
            o.mass = if function {
                MassKind::Function { state: false }
            } else {
                MassKind::Matrix(vec![0.])
            };
            let result = ode15s(
                &mut p,
                Setup {
                    tspan: &[0., 1.],
                    y0: &[1.],
                    f0: vec![-1.],
                    valt: None,
                    m0: function.then_some(vec![0.]),
                    at: OutAt::Requested,
                },
                &o,
            );
            let message = match result {
                Err(message) => message,
                Ok(out) => panic!("zero mass accepted: {:?} {:?}", out.stats, out.warnings),
            };
            println!("zero mass: {message}");
            assert_eq!(
                message,
                "If the mass matrix is all zero, the problem is not a differential algebraic equation."
            );
        }
    }

    #[test]
    fn ndf_fix1_dae_start_stays_inside_time_domain() {
        for backward in [false, true] {
            let mut p = Problem::new(if backward {
                |t, y| {
                    assert!((0. ..=1.).contains(&t), "t={t}");
                    vec![-y[0], y[1] - (1. - t).sqrt()]
                }
            } else {
                |t, y| {
                    assert!((0. ..=1.).contains(&t), "t={t}");
                    vec![-y[0], y[1] - t.sqrt()]
                }
            });
            let mut o = options(2, 1e-3);
            o.mass = MassKind::Matrix(vec![1., 0., 0., 0.]);
            let times = if backward {
                [1., 0.5, 0.]
            } else {
                [0., 0.5, 1.]
            };
            let out = run(&mut p, &times, &[1., 0.], &o, OutAt::Requested);
            println!("sqrt constraint: t={:?} stats={:?}", out.t, out.stats);
            assert_eq!(out.t, times);
            assert!(out.failure.is_none() && out.warnings.is_empty());
            for (&t, y) in out.t.iter().zip(&out.y) {
                assert!((y[0] - libm::exp(times[0] - t)).abs() < 0.01);
                let expected = if backward { (1. - t).sqrt() } else { t.sqrt() };
                assert!((y[1] - expected).abs() < 0.003);
            }
        }
    }

    #[test]
    fn ndf_fix1_consistent_slope_supports_general_singular_mass() {
        for kind in 0..3 {
            // y1=y2=exp(-t); the first equation is y1' (+ y2') = -y1 (- y2).
            let mut p = Problem::new(if kind == 2 {
                |_, y| vec![-y[0] - y[1], y[1] - y[0]]
            } else {
                |_, y| vec![-y[0], y[1] - y[0]]
            });
            let mut o = options(2, 1e-6);
            o.mass = if kind == 2 {
                MassKind::Matrix(vec![1., 0., 1., 0.])
            } else {
                p.mass = Some(|_, _| vec![1., 0., 0., 0.]);
                MassKind::Function { state: kind == 1 }
            };
            o.initial_slope = Some(vec![-1., -1.]);
            let out = run(&mut p, &[0., 0.5, 1.], &[1., 1.], &o, OutAt::Requested);
            println!(
                "consistent mass kind={kind}: y={:?} stats={:?}",
                out.y, out.stats
            );
            assert!(out.failure.is_none() && out.warnings.is_empty());
            assert_eq!(out.t, [0., 0.5, 1.]);
            assert_eq!(out.y[0], [1., 1.]);
            for (&t, y) in out.t.iter().zip(&out.y) {
                assert!(y.iter().all(|v| (v - libm::exp(-t)).abs() < 1e-5));
            }
            // Unsupported mass forms may not silently accept inconsistent data.
            for (state, initial) in [
                ([1., 1.], [-2., -1.]),
                ([1., 2.], [-1., -1.]),
                ([1., 1.], [f64::NAN, -1.]),
            ] {
                o.initial_slope = Some(initial.to_vec());
                let m0 = p.mass.map(|mass| mass(0., &state));
                let f0 = p.f(0., &state).unwrap();
                assert!(
                    ode15s(
                        &mut p,
                        Setup {
                            tspan: &[0., 1.],
                            y0: &state,
                            f0,
                            valt: None,
                            m0,
                            at: OutAt::Requested,
                        },
                        &o
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn ndf_fix1_initial_slope_is_ignored_for_odes() {
        for mass in [
            MassKind::None,
            MassKind::Matrix(vec![2.]),
            MassKind::Function { state: false },
        ] {
            let mut o = options(1, 1e-3);
            o.mass = mass;
            let mut p = Problem::new(decay);
            if matches!(o.mass, MassKind::Function { .. }) {
                p.mass = Some(|_, _| vec![2.]);
            }
            let expected = run(&mut p, &[0., 1.], &[1.], &o, OutAt::Sol);
            o.initial_slope = Some(vec![1e6]);
            let actual = run(&mut p, &[0., 1.], &[1.], &o, OutAt::Sol);
            println!(
                "ODE InitialSlope: baseline={} supplied={}",
                expected.stats[0], actual.stats[0]
            );
            assert_eq!(actual.t, expected.t);
            assert_eq!(actual.y, expected.y);
            assert_eq!(actual.stats, expected.stats);
        }
    }

    #[test]
    fn ndf_fix1_shared_numjac_threshold_is_abstol() {
        let mut o = options(2, 1e-3);
        o.atol = vec![1e-8, 2e-7];
        let jac = JacState::new(&o);
        println!("numjac threshold: {:?}", jac.thresh);
        assert_eq!(jac.thresh, o.atol);
    }

    #[test]
    fn ndf_fix1_shared_solvers_van_der_pol() {
        for solver in [ode23s, ode23t, ode23tb] {
            let mut p = Problem::new(|_, y| vec![y[1], 1000. * (1. - y[0] * y[0]) * y[1] - y[0]]);
            let mut o = options(2, 1e-3);
            o.hmax = 300.;
            o.htspan = 1500.;
            let f0 = p.f(0., &[2., 0.]).unwrap();
            let out = solver(
                &mut p,
                Setup {
                    tspan: &[0., 1500., 3000.],
                    y0: &[2., 0.],
                    f0,
                    valt: None,
                    m0: None,
                    at: OutAt::Requested,
                },
                &o,
            )
            .unwrap();
            println!("shared solver: y={:?} stats={:?}", out.y, out.stats);
            assert!(out.failure.is_none() && out.warnings.is_empty());
            assert_eq!(out.t, [0., 1500., 3000.]);
        }
    }

    #[test]
    fn ndf_polynomial_rescale_and_translate_all_orders_both_directions() {
        for k in 1..=5 {
            for h in [-0.2, 0.2] {
                let t = 0.75;
                let polynomial = |t: f64| vec![t.powi(k as i32), 1. + 0.3 * t.powi(k as i32)];
                let mut values: Vec<_> = (0..=k).map(|j| polynomial(t - j as f64 * h)).collect();
                let mut dif = vec![vec![0.; 2]; 8];
                for (j, column) in dif.iter_mut().enumerate().take(k + 1).skip(1) {
                    for l in 0..=k - j {
                        for i in 0..2 {
                            values[l][i] -= values[l + 1][i];
                        }
                    }
                    column.copy_from_slice(&values[0]);
                }
                for ratio in [0.1, 0.7, 2.5] {
                    for shift in [0., -0.4] {
                        let mut transformed = dif.clone();
                        ndf_rebase(&mut transformed, k, 5, shift, ratio);
                        let origin = t + shift * h;
                        for fraction in [0., -0.2, -0.5, -1.] {
                            let time = origin + fraction * ratio * h;
                            let actual = ntrp15s(
                                time,
                                origin,
                                &polynomial(origin),
                                ratio * h,
                                &transformed,
                                k,
                            );
                            let expected = polynomial(time);
                            for (a, b) in actual.iter().zip(expected) {
                                assert!(
                                    (a - b).abs() < 3e-10,
                                    "k={k} h={h} ratio={ratio} shift={shift}: {a} {b}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ndf_and_bdf_converge_with_each_maximum_order() {
        for bdf in [false, true] {
            for k in 1..=5 {
                let mut p = Problem::new(decay);
                let mut o = options(1, 1e-8);
                o.bdf = bdf;
                o.maxorder = k;
                o.jac = JacKind::Matrix(vec![-1.]);
                let out = run(&mut p, &[0., 1.], &[1.], &o, OutAt::Sol);
                check_counts(&p, &out);
                let bound = if k == 1 { 8e-5 } else { 2e-6 };
                for (&t, y) in out.t.iter().zip(&out.y) {
                    assert!(
                        (y[0] - libm::exp(-t)).abs() < bound,
                        "bdf={bdf} maxorder={k} t={t} y={y:?}"
                    );
                }
                if k >= 3 {
                    assert!(
                        out.data
                            .iter()
                            .any(|d| matches!(d, StepData::Ndf { k, .. } if *k >= 3))
                    );
                    assert!(out.stats[0] < 150, "{:?}", out.stats);
                    assert!(out.stats[4] < out.stats[0] / 2, "LU reuse: {:?}", out.stats);
                }
            }
        }
    }

    #[test]
    fn ndf_stiff_system_requested_output_and_analytic_jacobian_counts() {
        let mut p = Problem::new(|_, y| vec![-y[0], y[0] - 1000. * y[1]]);
        p.matrix = Some(|_, _| vec![-1., 1., 0., -1000.]);
        let mut o = options(2, 1e-7);
        // The engine permits either one AbsTol or one value per component.
        o.atol.truncate(1);
        o.jac = JacKind::Function;
        let out = run(
            &mut p,
            &[0., 0.0001, 0.01, 0.5, 1.],
            &[1., 1.],
            &o,
            OutAt::Requested,
        );
        assert_eq!(out.t, [0., 0.0001, 0.01, 0.5, 1.]);
        assert!(out.failure.is_none());
        assert_eq!(out.stats[2], p.calls);
        assert_eq!(out.stats[3], p.jac_calls);
        for (&t, y) in out.t.iter().zip(&out.y) {
            let slow = libm::exp(-t);
            let fast = libm::exp(-1000. * t) + (slow - libm::exp(-1000. * t)) / 999.;
            assert!(
                (y[0] - slow).abs() < 2e-6 && (y[1] - fast).abs() < 2e-6,
                "{t} {y:?}"
            );
        }
        assert!(out.stats[0] < 500, "{:?}", out.stats);
    }

    #[test]
    fn ndf_mass_functions_and_backward_normcontrol() {
        let mut p = Problem::new(decay);
        p.mass = Some(|t, _| vec![1. + t]);
        let mut o = options(1, 1e-8);
        o.mass = MassKind::Function { state: false };
        let out = run(&mut p, &[0., 0.3, 1.], &[1.], &o, OutAt::Requested);
        assert!(out.failure.is_none());
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - 1. / (1. + t)).abs() < 1e-6);
        }
        let mut p = Problem::new(|_, _| vec![-1.]);
        p.mass = Some(|_, y| vec![y[0]]);
        o.mass = MassKind::Function { state: true };
        let out = run(&mut p, &[0., 0.3, 1.], &[2.], &o, OutAt::Requested);
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - (4. - 2. * t).sqrt()).abs() < 2e-6);
        }
        let mut p = Problem::new(decay);
        let mut o = options(2, 1e-9);
        o.normcontrol = true;
        let out = run(&mut p, &[1., 0.7, 0.], &[1., 2.], &o, OutAt::Requested);
        for (&t, y) in out.t.iter().zip(&out.y) {
            for (i, value) in y.iter().enumerate() {
                assert!((value - (i + 1) as f64 * libm::exp(1. - t)).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn ndf_dae_initialization_includes_time_derivative_of_constraint() {
        let mut p = Problem::new(|t, y| vec![-y[0], y[1] - y[0] * y[0] - t]);
        let mut o = options(2, 1e-8);
        o.mass = MassKind::Matrix(vec![1., 0., 0., 0.]);
        let out = run(
            &mut p,
            &[0., 0.25, 0.5, 1.],
            &[1., -3.],
            &o,
            OutAt::Requested,
        );
        assert!(out.failure.is_none(), "{:?}", out.warnings);
        assert_eq!(out.y[0], [1., 1.]);
        for (&t, y) in out.t.iter().zip(&out.y) {
            assert!((y[0] - libm::exp(-t)).abs() < 1e-6, "{t} {y:?}");
            assert!((y[1] - libm::exp(-2. * t) - t).abs() < 2e-6, "{t} {y:?}");
        }
        let mut y = vec![1., -3.];
        let mut f = p.f(0., &y).unwrap();
        let mut jac = JacState::new(&o);
        let (yp, _) = daeic_diagonal(
            &mut p,
            &mut plain_f,
            &mut jac,
            None,
            0.,
            &[1., 0., 0., 0.],
            &mut y,
            &mut f,
            o.rtol,
            &o.threshold,
            &mut [0; 6],
        )
        .unwrap();
        assert!(
            (yp[0] + 1.).abs() < 1e-10 && (yp[1] + 1.).abs() < 1e-6,
            "{yp:?}"
        );
    }

    #[test]
    fn ndf_terminal_event_preserves_the_uncut_polynomial() {
        for dir in [-1., 1.] {
            let mut p = Problem::new(|t, _| vec![2. * t]);
            let mut o = options(1, 1e-8);
            o.htry = Some(1.);
            o.hmax = 1.;
            let full = run(&mut p, &[0., dir], &[0.], &o, OutAt::Sol);
            p.event = Some(|t, _| vec![t.abs() - 0.5]);
            o.events = true;
            let cut = run(&mut p, &[0., dir], &[0.], &o, OutAt::Sol);
            assert_eq!(*cut.t.last().unwrap(), dir * 0.5);
            let last = cut.t.len() - 1;
            let StepData::Ndf { k, dif } = &cut.data[last - 1] else {
                unreachable!()
            };
            let StepData::Ndf {
                k: full_k,
                dif: full_dif,
            } = &full.data[last - 1]
            else {
                unreachable!()
            };
            for theta in [0.1, 0.5, 0.9] {
                let t = cut.t[last - 1] + theta * (cut.t[last] - cut.t[last - 1]);
                let a = ntrp15s(
                    t,
                    cut.t[last],
                    &cut.y[last],
                    cut.t[last] - cut.t[last - 1],
                    dif,
                    *k,
                );
                let b = ntrp15s(
                    t,
                    full.t[last],
                    &full.y[last],
                    full.t[last] - full.t[last - 1],
                    full_dif,
                    *full_k,
                );
                assert!((a[0] - b[0]).abs() < 1e-12, "{t} {a:?} {b:?}");
                assert!((a[0] - t * t).abs() < 1e-8, "{t} {a:?}");
            }
        }
    }

    #[test]
    fn ndf_rejects_nonfinite_trials_and_honors_cancellation() {
        let mut p = Problem::new(|t, y| vec![-y[0], if t <= 0.5 { -y[1] } else { f64::NAN }]);
        let o = options(2, 1e-3);
        let out = run(&mut p, &[0., 1.], &[1., 1.], &o, OutAt::Sol);
        assert!(out.failure.is_some());
        assert!(
            (out.t.last().unwrap() - 0.5).abs() < 2e-14,
            "{:?}",
            out.t.last()
        );
        assert!(out.y.iter().flatten().all(|v| v.is_finite()));
        assert!(out.warnings[0].contains("t=5.000000e-01"));
        let mut p = Problem::new(decay);
        p.cancel_after = 5;
        let error = ode15s(
            &mut p,
            Setup {
                tspan: &[0., 1.],
                y0: &[1., 1.],
                f0: vec![-1., -1.],
                valt: None,
                m0: None,
                at: OutAt::Sol,
            },
            &o,
        )
        .err()
        .unwrap();
        assert_eq!(error, "cancelled");
    }

    #[test]
    fn ndf_nonlinear_stiff_oscillator_numeric_and_analytic_jacobians() {
        let mut answers = Vec::new();
        for tolerance in [1e-3, 1e-8] {
            for analytic in [false, true] {
                let mut p =
                    Problem::new(|_, y| vec![y[1], 1000. * (1. - y[0] * y[0]) * y[1] - y[0]]);
                let mut o = options(2, tolerance);
                o.htspan = 1500.;
                o.hmax = 300.;
                if analytic {
                    p.matrix = Some(|_, y| {
                        vec![
                            0.,
                            -2000. * y[0] * y[1] - 1.,
                            1.,
                            1000. * (1. - y[0] * y[0]),
                        ]
                    });
                    o.jac = JacKind::Function;
                }
                let out = run(&mut p, &[0., 1500., 3000.], &[2., 0.], &o, OutAt::Requested);
                assert!(out.failure.is_none(), "{:?}", out.warnings);
                assert!(out.stats[0] < 20000, "{:?}", out.stats);
                assert!(
                    out.stats[3] < out.stats[0],
                    "Jacobian reuse: {:?}",
                    out.stats
                );
                println!(
                    "NDF van der Pol rtol={tolerance} analytic={analytic}: y={:?} stats={:?}",
                    out.y, out.stats
                );
                answers.push(out.y);
            }
        }
        for row in 1..=2 {
            for component in 0..2 {
                let reference = answers[3][row][component];
                assert!(
                    (answers[2][row][component] - reference).abs()
                        < 2e-6 * reference.abs().max(1e-3)
                );
                for answer in &answers[..2] {
                    assert!(
                        (answer[row][component] - reference).abs() < 0.025 * reference.abs(),
                        "default={:?} tight={:?}",
                        answer[row],
                        answers[3][row]
                    );
                }
            }
        }
    }

    #[test]
    fn ndf_terminal_roots_restart_from_the_crossed_side() {
        for k in 1..200 {
            let mut p = Problem::new(|_, y| y.to_vec());
            p.event = Some(|_, y| vec![y[0]]);
            p.event_shift = 1. + k as f64 * 1.7 / 200.;
            let mut o = options(1, 1e-3);
            o.events = true;
            let first = run(&mut p, &[0., 1.], &[1.], &o, OutAt::Sol);
            assert_eq!(first.te.len(), 1);
            let te = first.te[0];
            let ye = first.ye[0][0];
            assert!(ye >= p.event_shift && ye - p.event_shift < 1e-12);
            assert_eq!(*first.t.last().unwrap(), te);
            let restarted = run(&mut p, &[te, 1.], &[ye], &o, OutAt::Sol);
            assert_eq!(*restarted.t.last().unwrap(), 1.);
            assert_eq!(restarted.te.len(), usize::from(ye == p.event_shift));
            assert!(restarted.te.iter().all(|&t| t == te));
        }
    }
}

/// Reciprocal error weights of a step from a to b: 1 / max(|a|, |b|, threshold),
/// or the scalar NormControl weight. Used by ode23s, ode23t and ode23tb.
fn stiff23_inverse_weights(o: &Opts, a: &[f64], b: &[f64]) -> Vec<f64> {
    if o.normcontrol {
        return vec![1. / mmax(mmax(norm2(a), norm2(b)), o.threshold[0]); a.len()];
    }
    (0..a.len())
        .map(|i| 1. / mmax(mmax(a[i].abs(), b[i].abs()), o.threshold[i]))
        .collect()
}

/// err^(-1/order) for the step-size controllers, through libm so every
/// platform takes the same steps.
fn stiff23_root(err: f64, order: f64) -> f64 {
    libm::pow(err, -1. / order)
}

/// Whether a step of this size cannot be taken at t: it would not move t, or
/// it is below the floor. A step that lands exactly on the endpoint is
/// allowed below the floor, since the remainder of the interval is fixed.
fn stiff23_step_too_small(t: f64, direction: f64, step: f64, minimum: f64, remaining: f64) -> bool {
    t + direction * step == t || (step < minimum && step != remaining)
}

/// Event location and output after an accepted step from t to tnew. Returns
/// the point where the step ends (an event time for a terminal event) and
/// whether integration stops there. `data` builds the sol interpolation data
/// for the (possibly cut) step ending at the given time and state.
#[allow(clippy::too_many_arguments)]
fn stiff23_finish(
    m: &mut dyn Model,
    out: &mut Out,
    s: &Setup,
    direction: f64,
    next: &mut usize,
    event_values: &mut Option<Vec<f64>>,
    t: f64,
    tnew: f64,
    ynew: &[f64],
    interpolate: &mut dyn FnMut(f64) -> Vec<f64>,
    data: &mut dyn FnMut(f64, &[f64]) -> StepData,
) -> R<(f64, Vec<f64>, bool)> {
    let mut finish_t = tnew;
    let mut finish_y = ynew.to_vec();
    let mut stop = false;
    if let Some(values) = event_values.as_ref() {
        let (found, right) = odezero(
            interpolate,
            &mut |time, state| m.events(time, state),
            s.tspan[0],
            values,
            t,
            tnew,
            ynew,
        )?;
        stop = found.stop;
        if stop {
            finish_t = *found.t.last().unwrap();
            finish_y = found.y.last().unwrap().clone();
        }
        out.add_events(&found);
        *event_values = Some(right);
    }
    let saved = if s.at == OutAt::Sol {
        Some(data(finish_t, &finish_y))
    } else {
        None
    };
    out.step_output(
        s.at,
        s.tspan,
        next,
        direction,
        t,
        finish_t,
        &finish_y,
        stop,
        saved,
        interpolate,
    )?;
    Ok((finish_t, finish_y, stop))
}

/// d = 1/(2 + sqrt(2)) of the modified Rosenbrock (2,3) pair.
const ROS23_D: f64 = 1. / (2. + std::f64::consts::SQRT_2);
/// e32 = 6 + sqrt(2) of the third stage.
const ROS23_E32: f64 = 6. + std::f64::consts::SQRT_2;

/// Continuous extension of the modified Rosenbrock pair, Shampine & Reichelt,
/// The MATLAB ODE Suite (1997), section 3:
/// y(t + s h) = y + h [s(1-s) k1 + s(s-2d) k2] / (1-2d).
pub fn ntrp23s(tinterp: f64, t: f64, y: &[f64], h: f64, k1: &[f64], k2: &[f64]) -> Vec<f64> {
    let s = (tinterp - t) / h;
    let scale = 1. - 2. * ROS23_D;
    let a = h * s * (1. - s) / scale;
    let b = h * s * (s - 2. * ROS23_D) / scale;
    (0..y.len()).map(|i| y[i] + a * k1[i] + b * k2[i]).collect()
}

/// Stages (k1, k2) of the same continuous extension on the shortened step
/// [t, t + theta*h]. The extension is p(tau) = y + tau*L/(1-2d) +
/// tau^2*D/(h(1-2d)) with L = k1 - 2d k2 and D = k2 - k1; keeping L and
/// h*D/h' fixed keeps the same quadratic.
fn ros23_rescale(theta: f64, k1: &[f64], k2: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let scale = 1. - 2. * ROS23_D;
    let mut a = Vec::with_capacity(k1.len());
    let mut b = Vec::with_capacity(k1.len());
    for i in 0..k1.len() {
        let linear = k1[i] - 2. * ROS23_D * k2[i];
        let quadratic = theta * (k2[i] - k1[i]);
        let second = (linear + quadratic) / scale;
        a.push(second - quadratic);
        b.push(second);
    }
    (a, b)
}

/// Modified Rosenbrock (2,3) pair as a W-method: Shampine & Reichelt, The
/// MATLAB ODE Suite (1997), section 3, with Hairer & Wanner, Solving Ordinary
/// Differential Equations II (1996), section IV.7. For M y' = f each step
/// factors W = M - h d J once and computes
///   W k1 = f0 + h d T,
///   W (k2 - k1) = f(t + h/2, y + h k1/2) - M k1,  ynew = y + h k2,
///   W k3 = f(tnew, ynew) - e32 (M k2 - f1) - 2 (M k1 - f0) + h d T,
/// with the local error estimate h (k1 - 2 k2 + k3) / 6 of the second-order
/// result. T = df/dt is a forward difference; J is formed at every step.
/// The step-size controller (safety factors, limits) is an implementation choice.
pub fn ode23s(m: &mut dyn Model, s: Setup, o: &Opts) -> R<Out> {
    m.check_cancel()?;
    let n = s.y0.len();
    let t0 = s.tspan[0];
    let end = *s.tspan.last().unwrap();
    let direction = sign(end - t0);
    let mut out = Out::new();
    out.stats[2] = 1; // The setup evaluated f(t0,y0).
    let mass = MassState::new(o, n, s.m0.clone());
    if mass.mtype >= 2 {
        return Err(
            "ODE23S cannot solve problems with non-constant mass matrices.  Use one of the other solvers of the ODE Suite."
                .into(),
        );
    }
    if mass.mtype == 1 && (mass.mt.iter().all(|v| *v == 0.) || is_dae(o, &mass.mt, n)) {
        return Err(
            "ODE23S cannot solve problems with singular mass matrices.  Use ode15s or ode23t."
                .into(),
        );
    }
    let mut jac = JacState::new(o);
    let jconst = match &o.jac {
        JacKind::Matrix(j) => Some(j.clone()),
        _ => None,
    };
    let mut t = t0;
    let mut y = s.y0.to_vec();
    let mut fvalue = s.f0.clone();
    out.push(t, y.clone())?;
    let mut event_values = s.valt.clone();
    let slope = if mass.mtype == 0 {
        fvalue.clone()
    } else {
        out.stats[4] += 1;
        out.stats[5] += 1;
        super::num::solve(&mass.mt, n, &fvalue)
    };
    let limit = mmin(o.hmax.min(f64::MAX), (end - t).abs());
    // Local errors behave like h^3: aim the first step at rtol^(1/3) of the
    // time scale |y| / |y'| measured in the error weights.
    let inverse: Vec<_> = weights(o, &y).iter().map(|w| 1. / w).collect();
    let speed = wnorm(o, &slope, &inverse);
    let mut absh = o
        .htry
        .unwrap_or_else(|| {
            if speed > 0. && speed.is_finite() {
                mmin(limit, libm::cbrt(o.rtol) / speed)
            } else {
                limit
            }
        })
        .min(limit)
        .max(16. * eps_of(t));
    let mut dfdy: Vec<f64> = Vec::new();
    let mut dfdt = vec![0.; n];
    let mut point_ready = false;
    let mut fresh_step = true;
    let mut failed = false;
    let mut next_output = 1;
    while direction * (end - t) > 0. {
        m.check_cancel()?;
        let minimum = 16. * eps_of(t);
        let remaining = (end - t).abs();
        // MaxStep (0.1 of a tiny interval by default) cannot push a step below
        // the floor of what the arithmetic resolves.
        let proposed = absh.min(mmax(o.hmax, minimum));
        let step = if fresh_step && 1.1 * proposed >= remaining {
            remaining
        } else {
            proposed.min(remaining)
        };
        if stiff23_step_too_small(t, direction, step, minimum, remaining) {
            out.failure = Some((t, minimum));
            out.warnings.push(failure_warning("ode23s", t, minimum));
            break;
        }
        absh = step;
        let tnew = if step == remaining {
            end
        } else {
            t + direction * step
        };
        let h = tnew - t;
        if !point_ready {
            // J and T at (t, y) serve every attempt from this point.
            dfdy = match &jconst {
                Some(j) => j.clone(),
                None if jac.constant && !dfdy.is_empty() => std::mem::take(&mut dfdy),
                None => jac.eval(m, &mut plain_f, t, &y, &fvalue, &mut out.stats)?,
            };
            let delta = (f64::EPSILON.sqrt() * mmax(t.abs(), step)).min(remaining);
            let tdel = if delta == remaining {
                end
            } else {
                t + direction * delta
            };
            m.check_cancel()?;
            let shifted = m.f(tdel, &y)?;
            out.stats[2] += 1;
            for i in 0..n {
                dfdt[i] = (shifted[i] - fvalue[i]) / (tdel - t);
            }
            point_ready = true;
        }
        let hd = h * ROS23_D;
        let matrix = iteration_matrix(&mass.mt, hd, &dfdy);
        let lu = Lu::new(&matrix, n);
        out.stats[4] += 1;
        let rhs: Vec<_> = (0..n).map(|i| fvalue[i] + hd * dfdt[i]).collect();
        let k1 = lu.solve(&rhs);
        out.stats[5] += 1;
        let half: Vec<_> = (0..n).map(|i| y[i] + 0.5 * h * k1[i]).collect();
        m.check_cancel()?;
        let f1 = m.f(t + 0.5 * h, &half)?;
        out.stats[2] += 1;
        let mk1 = mass.times(&k1);
        let rhs: Vec<_> = (0..n).map(|i| f1[i] - mk1[i]).collect();
        let k2: Vec<_> = lu.solve(&rhs).iter().zip(&k1).map(|(a, b)| a + b).collect();
        out.stats[5] += 1;
        let ynew = add(&y, &scale(h, &k2));
        m.check_cancel()?;
        let f2 = m.f(tnew, &ynew)?;
        out.stats[2] += 1;
        let mk2 = mass.times(&k2);
        let rhs: Vec<_> = (0..n)
            .map(|i| {
                f2[i] - ROS23_E32 * (mk2[i] - f1[i]) - 2. * (mk1[i] - fvalue[i]) + hd * dfdt[i]
            })
            .collect();
        let k3 = lu.solve(&rhs);
        out.stats[5] += 1;
        let estimate = scale(h / 6., &add(&sub(&k1, &scale(2., &k2)), &k3));
        let inverse = stiff23_inverse_weights(o, &y, &ynew);
        let err = wnorm(o, &estimate, &inverse) / o.rtol;
        let finite = err.is_finite() && ynew.iter().chain(&f2).all(|v| v.is_finite());
        if !finite || err > 1. {
            out.stats[1] += 1;
            let shrink = if finite {
                (0.8 * stiff23_root(err, 3.)).clamp(0.1, 0.8)
            } else {
                0.5
            };
            absh *= shrink;
            fresh_step = false;
            failed = true;
            continue;
        }
        out.stats[0] += 1;
        let mut interpolate = |time| ntrp23s(time, t, &y, h, &k1, &k2);
        let (finish_t, finish_y, stop) = stiff23_finish(
            m,
            &mut out,
            &s,
            direction,
            &mut next_output,
            &mut event_values,
            t,
            tnew,
            &ynew,
            &mut interpolate,
            &mut |time, _| {
                if time == tnew {
                    StepData::Ros {
                        k1: k1.clone(),
                        k2: k2.clone(),
                    }
                } else {
                    let (a, b) = ros23_rescale((time - t) / h, &k1, &k2);
                    StepData::Ros { k1: a, k2: b }
                }
            },
        )?;
        t = finish_t;
        y = finish_y;
        fvalue = f2;
        point_ready = false;
        fresh_step = true;
        if stop || t == end {
            break;
        }
        let mut ratio = if err == 0. {
            5.
        } else {
            (0.8 * stiff23_root(err, 3.)).min(5.)
        };
        if failed {
            ratio = ratio.min(1.);
        }
        failed = false;
        absh *= ratio;
    }
    Ok(out)
}

/// Quadratic through (t, y), (t2, y2) and (tnew, ynew): the continuous
/// extension of a TR-BDF2 step from its three solution values (Lagrange form;
/// Hairer, Norsett & Wanner, Solving Ordinary Differential Equations I (1993),
/// section II.6, continuous Runge-Kutta extensions).
pub fn ntrp23tb(
    tinterp: f64,
    t: f64,
    y: &[f64],
    tnew: f64,
    ynew: &[f64],
    t2: f64,
    y2: &[f64],
) -> Vec<f64> {
    let a = (tinterp - t2) * (tinterp - tnew) / ((t - t2) * (t - tnew));
    let b = (tinterp - t) * (tinterp - tnew) / ((t2 - t) * (t2 - tnew));
    let c = (tinterp - t) * (tinterp - t2) / ((tnew - t) * (tnew - t2));
    (0..y.len())
        .map(|i| a * y[i] + b * y2[i] + c * ynew[i])
        .collect()
}

/// Simplified Newton iteration shared by ode23t and ode23tb: solves
/// M*z - h*f(tz, y0 + c*z) = 0 for z starting from zpred; returns z, y and
/// whether the iteration converged.
/// Hairer & Wanner (1996), section IV.8: the iteration matrix M - c h J is
/// factored by the caller and kept while it converges. With the observed
/// contraction rate theta, the iteration stops when theta/(1-theta)*|dy| is
/// small compared with the tolerance, and is abandoned when it diverges
/// (theta >= 0.9) or when theta^(remaining)/(1-theta)*|dy| shows that the
/// remaining iterations cannot reach the tolerance. A rate is observed from
/// the second correction on; a single correction is accepted only at roundoff
/// level, since a stale Jacobian can map a large residual to a small change.
/// The last observed rate is reported through `rate` and `havrate`.
#[allow(clippy::too_many_arguments)]
fn newton_z(
    m: &mut dyn Model,
    o: &Opts,
    mass: &MassState,
    lu: &StiffLu,
    rowscale: Option<&Vec<f64>>,
    tz: f64,
    h: f64,
    base: &[f64],
    c: f64,
    zpred: Vec<f64>,
    invwt: &[f64],
    rate: &mut f64,
    havrate: &mut bool,
    stats: &mut [usize; 6],
) -> R<(Vec<f64>, Vec<f64>, bool)> {
    const ITERATIONS: i32 = 5;
    let n = base.len();
    let tolerance = mmax(0.03 * o.rtol, 10. * f64::EPSILON);
    let mut z = zpred;
    let mut y: Vec<f64> = (0..n).map(|i| base[i] + c * z[i]).collect();
    let mut previous = f64::INFINITY;
    for iteration in 0..ITERATIONS {
        m.check_cancel()?;
        if y.iter().any(|v| !v.is_finite()) {
            return Ok((z, y, false));
        }
        let fvalue = m.f(tz, &y)?;
        stats[2] += 1;
        if fvalue.iter().any(|v| !v.is_finite()) {
            return Ok((z, y, false));
        }
        let mz = mass.times(&z);
        let mut residual: Vec<f64> = (0..n).map(|i| h * fvalue[i] - mz[i]).collect();
        if let Some(scale) = rowscale {
            for (r, s) in residual.iter_mut().zip(scale) {
                *r *= s;
            }
        }
        let change = lu.solve(&residual);
        stats[5] += 1;
        // Size of the change in y = base + c z, in the error weights.
        let magnitude = c.abs() * wnorm(o, &change, invwt);
        if !magnitude.is_finite() {
            return Ok((z, y, false));
        }
        for i in 0..n {
            z[i] += change[i];
            y[i] = base[i] + c * z[i];
        }
        if magnitude <= 100. * f64::EPSILON {
            return Ok((z, y, true));
        }
        if iteration > 0 {
            *rate = magnitude / previous;
            *havrate = true;
            if *rate >= 0.9 {
                return Ok((z, y, false));
            }
            if magnitude * *rate / (1. - *rate) <= tolerance {
                return Ok((z, y, true));
            }
            let left = ITERATIONS - 1 - iteration;
            if magnitude * libm::pow(*rate, f64::from(left)) / (1. - *rate) > tolerance {
                return Ok((z, y, false));
            }
        }
        previous = magnitude;
    }
    Ok((z, y, false))
}

/// Starting values shared by ode23t and ode23tb: f0 after DAE initialization,
/// the initial slope, the Jacobian and the initial step (as in ode15s).
struct ImplicitStart {
    y: Vec<f64>,
    yp: Vec<f64>,
    dfdy: Vec<f64>,
    absh: f64,
    dae: bool,
}

/// Consistent start for M y' = f (Shampine, Reichelt & Kierzenka, Solving
/// Index-1 DAEs in MATLAB and Simulink (1999), sections 3–4): a supplied
/// consistent InitialSlope is used as is; otherwise ODEs take y' = M \ f and
/// DAEs with a constant diagonal mass matrix are made consistent. The first
/// step aims at the tolerance on the time scale |y| / |y'|, with the power of
/// rtol matching the order of the first error estimate.
fn implicit_start(
    m: &mut dyn Model,
    s: &Setup,
    o: &Opts,
    mass: &MassState,
    jac: &mut JacState,
    out: &mut Out,
    solver: &str,
) -> R<ImplicitStart> {
    let n = s.y0.len();
    let t = s.tspan[0];
    let end = *s.tspan.last().unwrap();
    let mut y = s.y0.to_vec();
    let mut fvalue = s.f0.clone();
    out.stats[2] = 1; // The setup evaluated f(t0,y0).
    if mass.mtype != 0 && mass.mt.iter().all(|v| *v == 0.) {
        return Err(
            "If the mass matrix is all zero, the problem is not a differential algebraic equation."
                .into(),
        );
    }
    let dae = mass.mtype != 0 && is_dae(o, &mass.mt, n);
    let jconst = match &o.jac {
        JacKind::Matrix(j) => Some(j),
        _ => None,
    };
    let (yp, dfdy) = if dae {
        let supplied = o.initial_slope.as_ref().filter(|initial| {
            if initial.len() != n || initial.iter().any(|v| !v.is_finite()) {
                return false;
            }
            let product = mass.times(initial);
            product.iter().zip(&fvalue).enumerate().all(|(i, (a, b))| {
                let atol = o.atol[if o.atol.len() == 1 { 0 } else { i }];
                a.is_finite()
                    && b.is_finite()
                    && (a - b).abs() <= atol + o.rtol * a.abs().max(b.abs())
            })
        });
        if let Some(initial) = supplied {
            let matrix = match jconst {
                Some(j) => j.clone(),
                None => jac.eval(m, &mut plain_f, t, &y, &fvalue, &mut out.stats)?,
            };
            (initial.clone(), matrix)
        } else {
            if mass.mtype != 1 {
                return Err(
                    "Consistent initial conditions require a constant diagonal mass matrix.".into(),
                );
            }
            daeic_diagonal(
                m,
                &mut plain_f,
                jac,
                jconst,
                t,
                &mass.mt,
                &mut y,
                &mut fvalue,
                o.rtol,
                &o.threshold,
                &mut out.stats,
            )?
        }
    } else {
        let derivative = if mass.mtype == 0 {
            fvalue.clone()
        } else {
            out.stats[4] += 1;
            out.stats[5] += 1;
            super::num::solve(&mass.mt, n, &fvalue)
        };
        let matrix = match jconst {
            Some(j) => j.clone(),
            None => jac.eval(m, &mut plain_f, t, &y, &fvalue, &mut out.stats)?,
        };
        (derivative, matrix)
    };
    out.push(t, y.clone())?;
    let inverse: Vec<_> = weights(o, &y).iter().map(|w| 1. / w).collect();
    let speed = wnorm(o, &yp, &inverse);
    let limit = o.hmax.min(f64::MAX).min((end - t).abs());
    // ode23t's first error estimate is first order (no step history yet).
    let target = if solver == "ode23t" {
        0.8 * o.rtol.sqrt()
    } else {
        libm::cbrt(o.rtol)
    };
    let absh = o
        .htry
        .unwrap_or_else(|| {
            if speed > 0. && speed.is_finite() {
                limit.min(target / speed)
            } else {
                limit
            }
        })
        .min(limit)
        .max(16. * eps_of(t));
    Ok(ImplicitStart {
        y,
        yp,
        dfdy,
        absh,
        dae,
    })
}

/// Partial-pivoting LU factors (nalgebra) of an iteration matrix, with the
/// sign of its determinant read off the pivots and the row permutation. The
/// sign is exact where the magnitude det(A) would underflow or overflow
/// (n = 120 and entries of 1e-3 give 1e-360).
struct StiffLu {
    lu: nalgebra::linalg::LU<f64, nalgebra::Dyn, nalgebra::Dyn>,
    sign: Option<i8>,
}

impl StiffLu {
    fn new(a: &[f64], n: usize) -> StiffLu {
        let lu = nalgebra::DMatrix::from_column_slice(n, n, a).lu();
        let sign = lu_determinant_sign(&lu);
        StiffLu { lu, sign }
    }

    /// Solution of A x = b; a singular A gives a nonfinite result, which
    /// the callers' convergence checks reject.
    fn solve(&self, b: &[f64]) -> Vec<f64> {
        let mut x = nalgebra::DVector::from_column_slice(b);
        if !self.lu.solve_mut(&mut x) {
            x.fill(f64::NAN);
        }
        x.as_slice().to_vec()
    }
}

/// Sign (-1, 0 or 1) of det(A) from the factors P A = L U: the parity of P
/// times the signs of the pivots of U. None when a pivot is NaN; 0 when a
/// pivot is exactly zero.
fn lu_determinant_sign(lu: &nalgebra::linalg::LU<f64, nalgebra::Dyn, nalgebra::Dyn>) -> Option<i8> {
    let mut sign = if lu.p().determinant::<f64>() < 0. {
        -1
    } else {
        1
    };
    let mut singular = false;
    let factors = lu.lu_internal();
    for i in 0..factors.nrows() {
        let pivot = factors[(i, i)];
        if pivot.is_nan() {
            return None;
        }
        singular |= pivot == 0.;
        if pivot < 0. {
            sign = -sign;
        }
    }
    Some(if singular { 0 } else { sign })
}

/// Sign of det(A) for a column-major n-by-n matrix.
fn matrix_determinant_sign(a: &[f64], n: usize) -> Option<i8> {
    lu_determinant_sign(&nalgebra::DMatrix::from_column_slice(n, n, a).lu())
}

/// Sign of det(M); the identity (no mass matrix) has +1.
fn mass_determinant_sign(mass: &MassState, n: usize) -> Option<i8> {
    if mass.mtype == 0 {
        Some(1)
    } else {
        matrix_determinant_sign(&mass.mt, n)
    }
}

/// Iteration matrix M - c h J of the implicit stages, row-scaled for DAEs,
/// factored once. Returns the factors and the row scaling.
fn stiff23_factor(
    mass: &MassState,
    ch: f64,
    dfdy: &[f64],
    dae: bool,
    n: usize,
    stats: &mut [usize; 6],
) -> (StiffLu, Option<Vec<f64>>) {
    let mut matrix = iteration_matrix(&mass.mt, ch, dfdy);
    let rows = if dae {
        let rows = row_scale(&matrix, n);
        scale_rows(&mut matrix, n, &rows);
        Some(rows)
    } else {
        None
    };
    stats[4] += 1;
    (StiffLu::new(&matrix, n), rows)
}

/// Whether det(M - c h J) has the sign of det(M), given the two signs. A
/// sign change means that a real eigenvalue of c h M^-1 J passed 1: the
/// trapezoidal rule then maps a growing mode to a decaying one and can follow
/// an unstable solution branch with a small error estimate, so such steps are
/// shortened. An exactly singular iteration matrix (an eigenvalue equal to 1)
/// also fails. With no orientation to keep (M singular or nonfinite) or a
/// nonfinite iteration matrix, the step is left to the Newton iteration.
fn tr_keeps_orientation(mass_sign: Option<i8>, iteration_sign: Option<i8>) -> bool {
    match (mass_sign, iteration_sign) {
        (Some(reference), Some(value)) if reference != 0 => value == reference,
        _ => true,
    }
}

/// Filtered local error x = (M - c h J)^-1 (M e - c h Z J e) of the raw
/// estimate e. Z selects the rows where M is zero, the algebraic constraints.
/// The other rows carry M e: the filter of Hairer & Wanner (1996), section
/// IV.8, which keeps the controller from reacting to the damped stiff modes.
/// A constraint row of the system reads -c h (J x)_i = rhs_i, so the right
/// side -c h (J e)_i makes x keep the constraint defect J_i e of the raw
/// estimate: the error of an algebraic component that the differential
/// components do not explain stays in the estimate, while the part propagated
/// from the differential components is damped with them. With plain M e a
/// constraint row has no defect, so an algebraic component such as y = t^3
/// would go uncontrolled.
fn stiff23_filter(
    mass: &MassState,
    lu: &StiffLu,
    rows: Option<&Vec<f64>>,
    dfdy: &[f64],
    ch: f64,
    estimate: &[f64],
    stats: &mut [usize; 6],
) -> Vec<f64> {
    let n = estimate.len();
    let mut rhs = mass.times(estimate);
    if mass.mtype != 0 {
        let constraints: Vec<usize> = (0..n)
            .filter(|&i| (0..n).all(|j| mass.mt[j * n + i] == 0.))
            .collect();
        if !constraints.is_empty() {
            let defect = matvec(dfdy, n, estimate);
            for i in constraints {
                rhs[i] = -ch * defect[i];
            }
        }
    }
    if let Some(scale) = rows {
        for (r, s) in rhs.iter_mut().zip(scale) {
            *r *= s;
        }
    }
    stats[5] += 1;
    lu.solve(&rhs)
}

/// Mass matrix M(t, y) or M(t) at a stage, when it is a function.
fn stiff23_mass(m: &mut dyn Model, mass: &mut MassState, t: f64, y: &[f64]) -> R<bool> {
    if mass.mtype >= 2 {
        mass.mt = mass.eval(m, t, y)?;
        return Ok(true);
    }
    Ok(false)
}

/// TR-BDF2: Bank, Coughran, Fichtner, Grosse, Rose & Smith, Transient
/// simulation of silicon devices and circuits (1985), with the analysis and
/// error estimate of Hosea & Shampine, Analysis and implementation of TR-BDF2
/// (1996). With gamma = 2 - sqrt(2), a trapezoidal stage reaches
/// t2 = t + gamma h and a BDF2 stage through t, t2 reaches tnew; both stages
/// solve M z - h f(., base + d z) = 0 for the scaled derivative z = h y' with
/// the same d = gamma/2, so one iteration matrix M - d h J serves both. As a
/// three-stage singly diagonally implicit Runge-Kutta method it has weights
/// (w, w, d), w = sqrt(2)/4, and the error estimate is the difference to the
/// third-order weights ((1-w)/3, (3w+1)/3, d/3) fixed by the quadrature
/// conditions on the nodes 0, gamma, 1. Jacobians are refreshed only when
/// the iteration fails to converge; step-size safety factors are
/// implementation choices.
pub fn ode23tb(m: &mut dyn Model, s: Setup, o: &Opts) -> R<Out> {
    m.check_cancel()?;
    let n = s.y0.len();
    let t0 = s.tspan[0];
    let end = *s.tspan.last().unwrap();
    let direction = sign(end - t0);
    let mut out = Out::new();
    let mut mass = MassState::new(o, n, s.m0.clone());
    let mut jac = JacState::new(o);
    let start = implicit_start(m, &s, o, &mass, &mut jac, &mut out, "ode23tb")?;
    let gamma = 2. - std::f64::consts::SQRT_2;
    let d = 0.5 * gamma;
    let w = 0.25 * std::f64::consts::SQRT_2;
    // Second-order weights minus third-order weights.
    let e0 = w - (1. - w) / 3.;
    let e2 = w - (3. * w + 1.) / 3.;
    let e1 = d - d / 3.;
    let bdf_y = 1. / (gamma * (2. - gamma));
    let bdf_y0 = (1. - gamma) * (1. - gamma) * bdf_y;
    let mut t = t0;
    let mut y = start.y;
    let mut yp = start.yp;
    let mut dfdy = start.dfdy;
    let mut absh = start.absh;
    let dae = start.dae;
    let mut event_values = s.valt.clone();
    if dae && o.events {
        let (v, term, dir) = m.events(t, &y)?;
        super::num::validate_events(&v, &term, &dir, event_values.as_ref().map_or(0, Vec::len))?;
        event_values = Some(v);
    }
    let mut factor: Option<(StiffLu, Option<Vec<f64>>)> = None;
    let mut factor_h = 0.;
    let mut jcurrent = true;
    let mut rate = 0.;
    let mut havrate = false;
    let mut fresh_step = true;
    let mut failed = false;
    let mut next_output = 1;
    while direction * (end - t) > 0. {
        m.check_cancel()?;
        let minimum = 16. * eps_of(t);
        let remaining = (end - t).abs();
        // MaxStep (0.1 of a tiny interval by default) cannot push a step below
        // the floor of what the arithmetic resolves.
        let proposed = absh.min(mmax(o.hmax, minimum));
        let step = if fresh_step && 1.1 * proposed >= remaining {
            remaining
        } else {
            proposed.min(remaining)
        };
        if stiff23_step_too_small(t, direction, step, minimum, remaining) {
            out.failure = Some((t, minimum));
            out.warnings.push(failure_warning("ode23tb", t, minimum));
            break;
        }
        absh = step;
        let tnew = if step == remaining {
            end
        } else {
            t + direction * step
        };
        let h = tnew - t;
        let t2 = t + gamma * h;
        let z0: Vec<f64> = yp.iter().map(|v| h * v).collect();
        // Trapezoidal stage, predicted with a constant derivative.
        let base2: Vec<f64> = (0..n).map(|i| y[i] + d * z0[i]).collect();
        let predicted2: Vec<f64> = (0..n).map(|i| base2[i] + d * z0[i]).collect();
        if stiff23_mass(m, &mut mass, t2, &predicted2)? || factor.is_none() || factor_h != h {
            factor = Some(stiff23_factor(&mass, d * h, &dfdy, dae, n, &mut out.stats));
            factor_h = h;
        }
        let inverse = stiff23_inverse_weights(o, &y, &predicted2);
        let (lu, rows) = factor.as_ref().unwrap();
        let (z2, y2, ok2) = newton_z(
            m,
            o,
            &mass,
            lu,
            rows.as_ref(),
            t2,
            h,
            &base2,
            d,
            z0.clone(),
            &inverse,
            &mut rate,
            &mut havrate,
            &mut out.stats,
        )?;
        let mut stage = None;
        if ok2 {
            // BDF2 stage, predicted by extrapolating the derivative linearly.
            let base1: Vec<f64> = (0..n).map(|i| bdf_y * y2[i] - bdf_y0 * y[i]).collect();
            let z1pred: Vec<f64> = (0..n).map(|i| z0[i] + (z2[i] - z0[i]) / gamma).collect();
            let predicted1: Vec<f64> = (0..n).map(|i| base1[i] + d * z1pred[i]).collect();
            if stiff23_mass(m, &mut mass, tnew, &predicted1)? {
                factor = Some(stiff23_factor(&mass, d * h, &dfdy, dae, n, &mut out.stats));
            }
            let inverse = stiff23_inverse_weights(o, &y, &predicted1);
            let (lu, rows) = factor.as_ref().unwrap();
            let (z1, ynew, ok1) = newton_z(
                m,
                o,
                &mass,
                lu,
                rows.as_ref(),
                tnew,
                h,
                &base1,
                d,
                z1pred,
                &inverse,
                &mut rate,
                &mut havrate,
                &mut out.stats,
            )?;
            if ok1 {
                stage = Some((z1, ynew));
            }
        }
        let Some((z1, ynew)) = stage else {
            if !jcurrent && !jac.constant {
                dfdy = jac.refresh(m, &mut plain_f, t, &y, &mut out.stats)?;
                jcurrent = true;
                havrate = false;
                factor = None;
            } else {
                out.stats[1] += 1;
                absh *= 0.3;
                factor = None;
                failed = true;
            }
            fresh_step = false;
            continue;
        };
        let raw: Vec<f64> = (0..n)
            .map(|i| e0 * z0[i] + e2 * z2[i] + e1 * z1[i])
            .collect();
        let (lu, rows) = factor.as_ref().unwrap();
        let estimate = stiff23_filter(&mass, lu, rows.as_ref(), &dfdy, d * h, &raw, &mut out.stats);
        let inverse = stiff23_inverse_weights(o, &y, &ynew);
        let err = wnorm(o, &estimate, &inverse) / o.rtol;
        if !err.is_finite() || err > 1. {
            out.stats[1] += 1;
            let shrink = if err.is_finite() {
                (0.8 * stiff23_root(err, 3.)).clamp(0.1, 0.8)
            } else {
                0.3
            };
            absh *= shrink;
            fresh_step = false;
            failed = true;
            continue;
        }
        out.stats[0] += 1;
        let mut interpolate = |time| ntrp23tb(time, t, &y, tnew, &ynew, t2, &y2);
        let (finish_t, finish_y, stop) = stiff23_finish(
            m,
            &mut out,
            &s,
            direction,
            &mut next_output,
            &mut event_values,
            t,
            tnew,
            &ynew,
            &mut interpolate,
            &mut |time, _| {
                if time == tnew {
                    StepData::TrBdf { t2, y2: y2.clone() }
                } else {
                    // Same quadratic on the cut step, sampled at its own
                    // intermediate node.
                    let mid = t + gamma * (time - t);
                    StepData::TrBdf {
                        t2: mid,
                        y2: ntrp23tb(mid, t, &y, tnew, &ynew, t2, &y2),
                    }
                }
            },
        )?;
        t = finish_t;
        y = finish_y;
        yp = z1.iter().map(|v| v / h).collect();
        fresh_step = true;
        jcurrent = false;
        if stop || t == end {
            break;
        }
        let mut ratio = if err == 0. {
            5.
        } else {
            (0.8 * stiff23_root(err, 3.)).min(5.)
        };
        if failed {
            ratio = ratio.min(1.);
        }
        failed = false;
        // Keep the factored iteration matrix through small increases.
        if !(1. ..1.2).contains(&ratio) {
            absh *= ratio;
        }
    }
    Ok(out)
}

/// Cubic Hermite interpolant of a trapezoidal step from the solution values
/// and the scaled derivatives z = h y'(t), znew = h y'(tnew) (Hairer, Norsett
/// & Wanner (1993), section II.6, dense output from Hermite data).
pub fn ntrp23t(
    tinterp: f64,
    t: f64,
    y: &[f64],
    h: f64,
    ynew: &[f64],
    z: &[f64],
    znew: &[f64],
) -> Vec<f64> {
    let s = (tinterp - t) / h;
    let s2 = s * s;
    let s3 = s2 * s;
    let h00 = 2. * s3 - 3. * s2 + 1.;
    let h01 = 3. * s2 - 2. * s3;
    let h10 = s3 - 2. * s2 + s;
    let h11 = s3 - s2;
    (0..y.len())
        .map(|i| h00 * y[i] + h01 * ynew[i] + h10 * z[i] + h11 * znew[i])
        .collect()
}

/// The trapezoidal rule for M y' = f, written for the scaled derivative
/// z = h y'(tnew): M z - h f(tnew, y + (z0 + z)/2) = 0 with z0 = h y'(t)
/// (Hairer, Norsett & Wanner (1993), section II.7; Hairer & Wanner (1996),
/// section IV.8 for the simplified Newton iteration). The local error
/// -h^3 y'''/12 is estimated, as in Milne's device (Hairer, Norsett & Wanner
/// (1993), section III.4), from the difference between the corrector and an
/// explicit second-order predictor that extrapolates y'' from the previous
/// step: y_p = y + h y' + h^2 (y' - y'_prev) / (2 h_prev), whose error is
/// h^2 (h/6 + h_prev/4) y''', so the trapezoidal error is
/// (ynew - y_p) h / (3 (h + h_prev)). The first step has no history and uses
/// the explicit Euler predictor (a first-order, conservative estimate).
/// Estimates are filtered through the iteration matrix (constraint rows keep
/// the defect of the raw estimate, see stiff23_filter); steps that would let h/2 J reverse a growing
/// mode, seen as a sign change of the iteration matrix's determinant taken
/// from its LU factors, are shortened (see tr_keeps_orientation).
/// Jacobians are refreshed when the iteration fails to converge or when a
/// secant check over the accepted step shows the kept one is out of date.
pub fn ode23t(m: &mut dyn Model, s: Setup, o: &Opts) -> R<Out> {
    m.check_cancel()?;
    let n = s.y0.len();
    let t0 = s.tspan[0];
    let end = *s.tspan.last().unwrap();
    let direction = sign(end - t0);
    let mut out = Out::new();
    let mut mass = MassState::new(o, n, s.m0.clone());
    let mut jac = JacState::new(o);
    let start = implicit_start(m, &s, o, &mass, &mut jac, &mut out, "ode23t")?;
    let mut t = t0;
    let mut y = start.y;
    let mut yp = start.yp;
    let mut dfdy = start.dfdy;
    let mut absh = start.absh;
    let dae = start.dae;
    // Sign of det(M) when M is constant (+1 without a mass matrix); a mass
    // function is re-examined at each factorization.
    let constant_sign = if dae || mass.mtype >= 2 {
        None
    } else {
        mass_determinant_sign(&mass, n)
    };
    let mut event_values = s.valt.clone();
    if dae && o.events {
        let (v, term, dir) = m.events(t, &y)?;
        super::num::validate_events(&v, &term, &dir, event_values.as_ref().map_or(0, Vec::len))?;
        event_values = Some(v);
    }
    // Derivative and signed length of the previous step, once one exists.
    let mut history: Option<(Vec<f64>, f64)> = None;
    let mut factor: Option<(StiffLu, Option<Vec<f64>>)> = None;
    let mut factor_h = 0.;
    let mut jcurrent = true;
    let mut rate = 0.;
    let mut havrate = false;
    let mut fresh_step = true;
    let mut failed = false;
    let mut next_output = 1;
    let mut stale = false;
    while direction * (end - t) > 0. {
        m.check_cancel()?;
        if stale {
            dfdy = jac.refresh(m, &mut plain_f, t, &y, &mut out.stats)?;
            jcurrent = true;
            havrate = false;
            factor = None;
            stale = false;
        }
        let minimum = 16. * eps_of(t);
        let remaining = (end - t).abs();
        // MaxStep (0.1 of a tiny interval by default) cannot push a step below
        // the floor of what the arithmetic resolves.
        let proposed = absh.min(mmax(o.hmax, minimum));
        let step = if fresh_step && 1.1 * proposed >= remaining {
            remaining
        } else {
            proposed.min(remaining)
        };
        if stiff23_step_too_small(t, direction, step, minimum, remaining) {
            out.failure = Some((t, minimum));
            out.warnings.push(failure_warning("ode23t", t, minimum));
            break;
        }
        absh = step;
        let tnew = if step == remaining {
            end
        } else {
            t + direction * step
        };
        let h = tnew - t;
        let z0: Vec<f64> = yp.iter().map(|v| h * v).collect();
        let predicted: Vec<f64> = match &history {
            Some((previous, hp)) => (0..n)
                .map(|i| y[i] + h * yp[i] + 0.5 * h * h / hp * (yp[i] - previous[i]))
                .collect(),
            None => (0..n).map(|i| y[i] + z0[i]).collect(),
        };
        let base: Vec<f64> = (0..n).map(|i| y[i] + 0.5 * z0[i]).collect();
        let zpred: Vec<f64> = (0..n).map(|i| 2. * (predicted[i] - y[i]) - z0[i]).collect();
        if stiff23_mass(m, &mut mass, tnew, &predicted)? || factor.is_none() || factor_h != h {
            let fresh = stiff23_factor(&mass, 0.5 * h, &dfdy, dae, n, &mut out.stats);
            if !dae {
                let reference = if mass.mtype >= 2 {
                    mass_determinant_sign(&mass, n)
                } else {
                    constant_sign
                };
                if !tr_keeps_orientation(reference, fresh.0.sign) {
                    out.stats[1] += 1;
                    absh *= 0.5;
                    factor = None;
                    fresh_step = false;
                    failed = true;
                    continue;
                }
            }
            factor = Some(fresh);
            factor_h = h;
        }
        let inverse = stiff23_inverse_weights(o, &y, &predicted);
        let (lu, rows) = factor.as_ref().unwrap();
        let (znew, ynew, converged) = newton_z(
            m,
            o,
            &mass,
            lu,
            rows.as_ref(),
            tnew,
            h,
            &base,
            0.5,
            zpred,
            &inverse,
            &mut rate,
            &mut havrate,
            &mut out.stats,
        )?;
        if !converged {
            if !jcurrent && !jac.constant {
                dfdy = jac.refresh(m, &mut plain_f, t, &y, &mut out.stats)?;
                jcurrent = true;
                havrate = false;
            } else {
                out.stats[1] += 1;
                absh *= 0.3;
                failed = true;
            }
            factor = None;
            fresh_step = false;
            continue;
        }
        let (order, coefficient) = match &history {
            Some((_, hp)) => (3., h / (3. * (h + hp))),
            None => (2., 1.),
        };
        let raw = scale(coefficient, &sub(&ynew, &predicted));
        let (lu, rows) = factor.as_ref().unwrap();
        let estimate = stiff23_filter(
            &mass,
            lu,
            rows.as_ref(),
            &dfdy,
            0.5 * h,
            &raw,
            &mut out.stats,
        );
        let inverse = stiff23_inverse_weights(o, &y, &ynew);
        let err = wnorm(o, &estimate, &inverse) / o.rtol;
        if !err.is_finite() || err > 1. {
            out.stats[1] += 1;
            let shrink = if err.is_finite() {
                (0.8 * stiff23_root(err, order)).clamp(0.1, 0.8)
            } else {
                0.3
            };
            absh *= shrink;
            fresh_step = false;
            failed = true;
            continue;
        }
        out.stats[0] += 1;
        let mut interpolate = |time| ntrp23t(time, t, &y, h, &ynew, &z0, &znew);
        let (finish_t, finish_y, stop) = stiff23_finish(
            m,
            &mut out,
            &s,
            direction,
            &mut next_output,
            &mut event_values,
            t,
            tnew,
            &ynew,
            &mut interpolate,
            &mut |time, _| {
                if time == tnew {
                    StepData::Tr {
                        z: z0.clone(),
                        znew: znew.clone(),
                    }
                } else {
                    // Same cubic on the cut step: rescale both derivatives
                    // to the new step length.
                    let theta = (time - t) / h;
                    let mut cut = vec![0.; n];
                    for i in 0..n {
                        let delta = ynew[i] - y[i];
                        let slope = z0[i]
                            + 2. * theta * (3. * delta - 2. * z0[i] - znew[i])
                            + 3. * theta * theta * (z0[i] + znew[i] - 2. * delta);
                        cut[i] = theta * slope;
                    }
                    StepData::Tr {
                        z: z0.iter().map(|v| theta * v).collect(),
                        znew: cut,
                    }
                }
            },
        )?;
        let accepted_yp: Vec<f64> = znew.iter().map(|v| v / h).collect();
        if !jac.constant {
            // Secant check of the kept Jacobian: over the step f changed by
            // M (y'new - y'), which J (ynew - y) should roughly reproduce.
            let change = mass.times(&sub(&accepted_yp, &yp));
            let linear = matvec(&dfdy, n, &sub(&ynew, &y));
            let size = mmax(
                norm_inf(change.iter().copied()),
                norm_inf(linear.iter().copied()),
            );
            stale = size > 0. && norm_inf(sub(&change, &linear)) > 0.5 * size;
        }
        history = Some((std::mem::replace(&mut yp, accepted_yp), h));
        t = finish_t;
        y = finish_y;
        fresh_step = true;
        jcurrent = false;
        if stop || t == end {
            break;
        }
        let mut ratio = if err == 0. {
            5.
        } else {
            (0.8 * stiff23_root(err, order)).min(5.)
        };
        if failed {
            ratio = ratio.min(1.);
        }
        failed = false;
        if !(1. ..1.2).contains(&ratio) {
            absh *= ratio;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod stiff23_tests {
    use super::*;

    type Rhs = fn(f64, &[f64]) -> Vec<f64>;

    struct Problem {
        rhs: Rhs,
        calls: usize,
    }
    impl Model for Problem {
        fn f(&mut self, t: f64, y: &[f64]) -> R<Vec<f64>> {
            self.calls += 1;
            Ok((self.rhs)(t, y))
        }
        fn jac(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            unreachable!()
        }
        fn mass(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            unreachable!()
        }
        fn events(&mut self, _: f64, _: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
            unreachable!()
        }
        fn check_cancel(&self) -> R<()> {
            Ok(())
        }
    }

    type Solver = fn(&mut dyn Model, Setup, &Opts) -> R<Out>;
    const SOLVERS: [(&str, Solver); 3] =
        [("ode23s", ode23s), ("ode23t", ode23t), ("ode23tb", ode23tb)];

    fn options(n: usize, rtol: f64, atol: f64) -> Opts {
        Opts {
            rtol,
            atol: vec![atol; n],
            normcontrol: false,
            threshold: vec![atol / rtol; n],
            hmax: 0.1,
            htry: None,
            htspan: 1.,
            maxorder: 5,
            bdf: false,
            jac: JacKind::Numeric,
            jconstant: false,
            mass: MassKind::None,
            mass_singular: "maybe".into(),
            initial_slope: None,
            events: false,
        }
    }

    fn run(solver: Solver, rhs: Rhs, times: &[f64], y: &[f64], o: &Opts, at: OutAt) -> Out {
        let mut p = Problem { rhs, calls: 0 };
        let f0 = rhs(times[0], y);
        let out = solver(
            &mut p,
            Setup {
                tspan: times,
                y0: y,
                f0,
                valt: None,
                m0: None,
                at,
            },
            o,
        )
        .unwrap();
        // The setup's f(t0, y0) is counted once, like every other call.
        assert_eq!(out.stats[2], p.calls + 1);
        if at == OutAt::Steps {
            assert_eq!(out.stats[0], out.t.len() - 1);
        }
        out
    }

    #[test]
    fn tr_bdf2_weights_and_estimate_follow_the_quadrature_conditions() {
        let gamma = 2. - std::f64::consts::SQRT_2;
        let d = 0.5 * gamma;
        let w = 0.25 * std::f64::consts::SQRT_2;
        let nodes = [0., gamma, 1.];
        // BDF2 stage expanded: y1 = y0 + h (w y'0 + w y'2 + d y'1).
        assert!((d / (gamma * (2. - gamma)) - w).abs() < 1e-15);
        assert!(((1. - gamma) / (2. - gamma) - d).abs() < 1e-15);
        let low = [w, w, d];
        let high = [(1. - w) / 3., (3. * w + 1.) / 3., d / 3.];
        for power in 0..3 {
            let moment = |b: &[f64; 3]| (0..3).map(|i| b[i] * nodes[i].powi(power)).sum::<f64>();
            assert!((moment(&high) - 1. / f64::from(power + 1)).abs() < 1e-15);
            if power < 2 {
                assert!((moment(&low) - 1. / f64::from(power + 1)).abs() < 1e-15);
            }
        }
    }

    #[test]
    fn continuous_extensions_reproduce_their_polynomials_on_cut_steps() {
        let (t, h) = (0.25, -0.5);
        let y = [1., -2.];
        let (k1, k2) = ([0.3, -0.7], [1.1, 0.4]);
        for theta in [0.2, 0.6, 1.] {
            let (a, b) = ros23_rescale(theta, &k1, &k2);
            for s in [0.1, 0.5, 0.9] {
                let x = t + s * theta * h;
                let p = ntrp23s(x, t, &y, h, &k1, &k2);
                let q = ntrp23s(x, t, &y, theta * h, &a, &b);
                assert!(
                    p.iter().zip(&q).all(|(u, v)| (u - v).abs() < 1e-14),
                    "{p:?} {q:?}"
                );
            }
        }
        // The Hermite cubic interpolates values and scaled slopes.
        let (ynew, z, znew) = ([0.5, 3.], [0.2, -1.], [-0.4, 2.]);
        assert_eq!(ntrp23t(t, t, &y, h, &ynew, &z, &znew), y);
        assert_eq!(ntrp23t(t + h, t, &y, h, &ynew, &z, &znew), ynew);
        // A quadratic through three nodes is exact for t^2.
        let q = ntrp23tb(0.3, 0., &[0.], 1., &[1.], 0.6, &[0.36]);
        assert!((q[0] - 0.09).abs() < 1e-15);
    }

    #[test]
    fn each_solver_converges_with_the_tolerance_on_a_forced_stiff_problem() {
        // y1' = -y1, y2' = -1000 (y2 - cos t): y2 tracks cos t + sin t / 1000.
        fn rhs(t: f64, y: &[f64]) -> Vec<f64> {
            vec![-y[0], -1000. * (y[1] - libm::cos(t))]
        }
        let exact2 = |t: f64| {
            let c = 1000. / (1000. * 1000. + 1.);
            1000. * c * libm::cos(t) + c * libm::sin(t) - 1000. * c * libm::exp(-1000. * t)
        };
        for (name, solver) in SOLVERS {
            let mut errors = Vec::new();
            for rtol in [1e-3, 1e-5, 1e-7] {
                let o = options(2, rtol, rtol * 1e-3);
                let out = run(solver, rhs, &[0., 1.], &[1., 0.], &o, OutAt::Steps);
                assert!(out.failure.is_none(), "{name}: {:?}", out.warnings);
                let last = out.y.last().unwrap();
                let error = mmax(
                    (last[0] - libm::exp(-1.)).abs(),
                    (last[1] - exact2(1.)).abs(),
                );
                assert!(error < 30. * rtol, "{name} rtol={rtol}: {error:e}");
                assert!(out.t.len() < 3000, "{name}: {} points", out.t.len());
                errors.push(error);
            }
            assert!(errors[2] < errors[0] / 50., "{name}: {errors:?}");
        }
    }

    #[test]
    fn requested_output_and_reverse_time_match_the_closed_form() {
        fn rhs(t: f64, y: &[f64]) -> Vec<f64> {
            vec![y[1], -y[0] + 0. * t]
        }
        for (name, solver) in SOLVERS {
            let o = options(2, 1e-6, 1e-9);
            let times = [2., 1.5, 1., 0.25];
            let y0 = [libm::sin(2.), libm::cos(2.)];
            let out = run(solver, rhs, &times, &y0, &o, OutAt::Requested);
            assert_eq!(out.t, times);
            for (t, y) in out.t.iter().zip(&out.y) {
                assert!((y[0] - libm::sin(*t)).abs() < 5e-5, "{name} {t}: {y:?}");
                assert!((y[1] - libm::cos(*t)).abs() < 5e-5, "{name} {t}: {y:?}");
            }
        }
    }

    #[test]
    fn trapezoidal_steps_keep_growing_modes_growing() {
        // With h/2 * 50 > 1 the trapezoidal rule would turn growth into decay.
        // Ten e-foldings at RelTol 1e-3 allow a few percent of global error.
        fn rhs(_: f64, y: &[f64]) -> Vec<f64> {
            vec![50. * y[0]]
        }
        let mut o = options(1, 1e-3, 1e-6);
        o.hmax = 1.;
        o.htry = Some(1.);
        let out = run(ode23t, rhs, &[0., 0.2], &[1.], &o, OutAt::Steps);
        let last = out.y.last().unwrap()[0];
        assert!(
            ((last - libm::exp(10.)) / libm::exp(10.)).abs() < 5e-2,
            "{last}"
        );
        assert!(out.y.windows(2).all(|w| w[1][0] > w[0][0]));
    }

    #[test]
    fn non_constant_and_singular_mass_matrices_are_rejected_by_ode23s() {
        let mut o = options(1, 1e-3, 1e-6);
        o.mass = MassKind::Function { state: false };
        let mut p = Problem {
            rhs: |_, y| vec![-y[0]],
            calls: 0,
        };
        let setup = |m0| Setup {
            tspan: &[0., 1.],
            y0: &[1.],
            f0: vec![-1.],
            valt: None,
            m0,
            at: OutAt::Steps,
        };
        let error = ode23s(&mut p, setup(Some(vec![1.])), &o).err().unwrap();
        assert!(error.contains("non-constant mass"), "{error}");
        o.mass = MassKind::Matrix(vec![0.]);
        let error = ode23s(&mut p, setup(None), &o).err().unwrap();
        assert!(error.contains("singular mass"), "{error}");
    }

    fn scaled_identity(n: usize, value: f64) -> Vec<f64> {
        let mut a = vec![0.; n * n];
        for i in 0..n {
            a[i * n + i] = value;
        }
        a
    }

    #[test]
    fn determinant_signs_come_from_pivots_and_permutations() {
        // det(1e-3 I) = 1e-360 underflows; the sign does not.
        assert_eq!(
            matrix_determinant_sign(&scaled_identity(120, 1e-3), 120),
            Some(1)
        );
        assert_eq!(
            matrix_determinant_sign(&scaled_identity(120, -1e-3), 120),
            Some(1)
        );
        assert_eq!(
            matrix_determinant_sign(&scaled_identity(121, -1e-3), 121),
            Some(-1)
        );
        assert_eq!(
            matrix_determinant_sign(&scaled_identity(60, 1e300), 60),
            Some(1)
        );
        // A row swap alone gives -1; a third swap restores the parity.
        assert_eq!(matrix_determinant_sign(&[0., 1., 1., 0.], 2), Some(-1));
        let cycle = [0., 1., 0., 0., 0., 1., 1., 0., 0.];
        assert_eq!(matrix_determinant_sign(&cycle, 3), Some(1));
        // Exactly singular, and not a number.
        assert_eq!(matrix_determinant_sign(&[1., 2., 2., 4.], 2), Some(0));
        assert_eq!(matrix_determinant_sign(&[0.; 4], 2), Some(0));
        assert_eq!(matrix_determinant_sign(&[1., f64::NAN, 0., 1.], 2), None);

        // Orientation: equal signs keep it, a flip or a singular iteration
        // matrix does not; a singular or nonfinite M leaves nothing to keep.
        assert!(tr_keeps_orientation(Some(1), Some(1)));
        assert!(tr_keeps_orientation(Some(-1), Some(-1)));
        assert!(!tr_keeps_orientation(Some(1), Some(-1)));
        assert!(!tr_keeps_orientation(Some(-1), Some(1)));
        assert!(!tr_keeps_orientation(Some(1), Some(0)));
        assert!(tr_keeps_orientation(Some(0), Some(-1)));
        assert!(tr_keeps_orientation(Some(0), Some(0)));
        assert!(tr_keeps_orientation(None, Some(-1)));
        assert!(tr_keeps_orientation(Some(1), None));
    }

    #[test]
    fn a_small_mass_matrix_of_dimension_120_does_not_stop_the_solvers() {
        // det(M) = 1e-360 underflows to zero; the solvers must not read that
        // as an orientation change and halve the step to the floor.
        fn decay(_: f64, y: &[f64]) -> Vec<f64> {
            y.iter().map(|v| -v).collect()
        }
        fn growth(_: f64, y: &[f64]) -> Vec<f64> {
            y.iter().map(|v| 0.05 * v).collect()
        }
        let n = 120;
        for (name, solver) in SOLVERS {
            let mut o = options(n, 1e-3, 1e-6);
            o.mass = MassKind::Matrix(scaled_identity(n, 1e-3));
            let out = run(solver, decay, &[0., 1.], &vec![1.; n], &o, OutAt::Steps);
            assert!(out.failure.is_none(), "{name}: {:?}", out.warnings);
            assert_eq!(*out.t.last().unwrap(), 1., "{name}");
            assert!(out.t.len() < 100, "{name}: {} points", out.t.len());
            assert!(
                out.y.last().unwrap().iter().all(|v| v.abs() < 1e-3),
                "{name}"
            );
        }
        // The growing mode (y' = 50 y after dividing by M) must stay growing.
        let mut o = options(n, 1e-3, 1e-6);
        o.mass = MassKind::Matrix(scaled_identity(n, 1e-3));
        o.hmax = 1.;
        o.htry = Some(1.);
        let out = run(ode23t, growth, &[0., 0.2], &vec![1.; n], &o, OutAt::Steps);
        assert!(out.failure.is_none(), "{:?}", out.warnings);
        let last = out.y.last().unwrap()[0];
        assert!(
            ((last - libm::exp(10.)) / libm::exp(10.)).abs() < 5e-2,
            "{last}"
        );
        assert!(out.y.windows(2).all(|w| w[1][0] > w[0][0]));
    }

    #[test]
    fn a_final_remainder_below_the_step_floor_is_taken() {
        // 1e-15 is representable but below 16 eps(1): the last step may be
        // that short, as it only lands on the endpoint.
        fn rhs(_: f64, y: &[f64]) -> Vec<f64> {
            y.iter().map(|v| 0. * v).collect()
        }
        for (name, solver) in SOLVERS {
            for end in [1. + 1e-15, 1. - 1e-15] {
                let mut o = options(1, 1e-3, 1e-6);
                o.hmax = 0.1 * (end - 1f64).abs(); // the default MaxStep
                let out = run(solver, rhs, &[1., end], &[1.], &o, OutAt::Steps);
                assert!(out.failure.is_none(), "{name} {end}: {:?}", out.warnings);
                assert_eq!(out.t, [1., end], "{name}");
                assert!(out.y.iter().all(|y| (y[0] - 1.).abs() < 1e-14), "{name}");
            }
        }
        // A step that cannot land on the endpoint still counts as too small.
        assert!(stiff23_step_too_small(1., 1., 1e-15, 3.5e-15, 2e-15));
        assert!(!stiff23_step_too_small(1., 1., 1e-15, 3.5e-15, 1e-15));
        assert!(stiff23_step_too_small(1., 1., 1e-17, 1e-17, 1e-17));
    }

    #[test]
    fn algebraic_components_stay_under_error_control() {
        // M = diag(1, 0): y1' = 0 and 0 = y2 - t^3. The cubic Hermite and
        // quadratic interpolants inside each step follow t^3 more closely as
        // RelTol tightens only if the algebraic component is controlled.
        fn rhs(t: f64, y: &[f64]) -> Vec<f64> {
            vec![0., y[1] - t * t * t]
        }
        for (name, solver) in [("ode23t", ode23t as Solver), ("ode23tb", ode23tb)] {
            let mut worst = Vec::new();
            for rtol in [1e-3, 1e-6] {
                let mut o = options(2, rtol, 1e-9);
                o.mass = MassKind::Matrix(vec![1., 0., 0., 0.]);
                let out = run(solver, rhs, &[0., 1.], &[1., 0.], &o, OutAt::Sol);
                assert!(out.failure.is_none(), "{name}: {:?}", out.warnings);
                assert_eq!(out.data.len(), out.t.len() - 1, "{name}");
                let mut error = 0f64;
                for (i, data) in out.data.iter().enumerate() {
                    let (t, tnew) = (out.t[i], out.t[i + 1]);
                    let mid = 0.5 * (t + tnew);
                    let value = match data {
                        StepData::Tr { z, znew } => {
                            ntrp23t(mid, t, &out.y[i], tnew - t, &out.y[i + 1], z, znew)
                        }
                        StepData::TrBdf { t2, y2 } => {
                            ntrp23tb(mid, t, &out.y[i], tnew, &out.y[i + 1], *t2, y2)
                        }
                        _ => unreachable!("{name}"),
                    };
                    error = mmax(error, (value[1] - mid * mid * mid).abs());
                }
                println!(
                    "{name} rtol={rtol}: {} points, midpoint error {error:e}",
                    out.t.len()
                );
                worst.push(error);
            }
            // 1.25e-4 before the constraint rows kept their error estimate.
            assert!(worst[0] < 1e-4, "{name}: {worst:?}");
            assert!(worst[1] < 1e-6, "{name}: {worst:?}");
            assert!(worst[1] < worst[0] / 20., "{name}: {worst:?}");
        }
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    struct Analytic;
    impl Model for Analytic {
        fn f(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            panic!("analytic Jacobian refresh must not evaluate f(t,y)");
        }
        fn jac(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            Ok(vec![-1.])
        }
        fn mass(&mut self, _: f64, _: &[f64]) -> R<Vec<f64>> {
            unreachable!()
        }
        fn events(&mut self, _: f64, _: &[f64]) -> R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
            unreachable!()
        }
        fn check_cancel(&self) -> R<()> {
            Ok(())
        }
    }

    #[test]
    fn analytic_refresh_counts_only_the_jacobian() {
        let mut jac = JacState {
            constant: false,
            analytic: true,
            thresh: vec![1e-6],
            fac: Vec::new(),
        };
        let mut stats = [0; 6];
        let result = jac
            .refresh(
                &mut Analytic,
                &mut |m, t, y| m.f(t, y),
                1.,
                &[1.],
                &mut stats,
            )
            .unwrap();
        assert_eq!(result, vec![-1.]);
        assert_eq!(stats, [0, 0, 0, 1, 0, 0]);
    }

    #[test]
    fn interpolation_storage_drops_stages_at_the_limit() {
        let mut out = Out::new();
        let data = || StepData::Abm {
            phi: vec![vec![1.; 2]; 14],
            psi: vec![1.; 12],
            k: 1,
        };
        out.push_data(data(), 50);
        assert_eq!(out.data.len(), 1);
        out.push_data(data(), 50);
        assert!(out.data_dropped);
        assert!(out.data.is_empty());
        assert_eq!(out.data_elements, 0);
        out.push_data(data(), 50);
        assert!(out.data.is_empty());
    }

    #[test]
    fn terminal_abm_rebase_preserves_each_supported_order_and_direction() {
        for k in 1..=12 {
            for dir in [-1., 1.] {
                let mut psi: Vec<f64> = (0..14).map(|j| dir * j as f64 * 0.8).collect();
                let mut phi: Vec<Vec<f64>> = (0..15)
                    .map(|j| vec![0.01f64.powi(j), 0.02f64.powi(j)])
                    .collect();
                let old_phi = phi.clone();
                let old_psi = psi.clone();
                let y = [1., 2.];
                let te = -dir * 0.35;
                let ye = ntrp113(te, 0., &y, k, &phi, &psi);
                rebase113(te, k, &mut phi, &mut psi);
                for t in [-dir * 0.7, -dir * 0.5, te] {
                    let expected = ntrp113(t, 0., &y, k, &old_phi, &old_psi);
                    let actual = ntrp113(t, te, &ye, k, &phi, &psi);
                    assert!(
                        actual
                            .iter()
                            .zip(&expected)
                            .all(|(a, b)| (a - b).abs() < 1e-10),
                        "order={k} direction={dir}: {actual:?} != {expected:?}"
                    );
                }
            }
        }
    }
}
