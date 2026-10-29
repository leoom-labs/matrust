//! Embedded Runge-Kutta pairs of Dormand & Prince (1980) and Bogacki & Shampine
//! (1989), with scaled maximum-norm control (Shampine & Reichelt, 1997, section 2)
//! and continuous output (Shampine, 1986), plus odeset/odeget.
//! Option sets are 1-by-1 structs with odeset's 22 fields; unset fields are [].
use super::{Engine, Execution, Value};
use crate::{
    Array, CharacterRow, FunctionHandle, Matrix, Result, StructArray, matrix::MAX_ELEMENTS,
};
use std::sync::Arc;

/// odeset property names (reference order). Names match by case-insensitive prefix.
const ODE_NAMES: [&str; 22] = [
    "AbsTol",
    "BDF",
    "Events",
    "InitialStep",
    "Jacobian",
    "JConstant",
    "JPattern",
    "Mass",
    "MassSingular",
    "MaxOrder",
    "MaxStep",
    "NonNegative",
    "NormControl",
    "OutputFcn",
    "OutputSel",
    "Refine",
    "RelTol",
    "Stats",
    "Vectorized",
    "MStateDependence",
    "MvPattern",
    "InitialSlope",
];
const ODE_SUPPORTED: [&str; 5] = ["AbsTol", "InitialStep", "MaxStep", "Refine", "RelTol"];

pub(super) fn is_solver(name: &str) -> bool {
    matches!(name, "ode45" | "ode23")
}

#[derive(Clone, Copy, PartialEq)]
enum OdeMethod {
    DormandPrince45,
    BogackiShampine23,
}
impl OdeMethod {
    fn pow(self) -> f64 {
        match self {
            Self::DormandPrince45 => 1. / 5.,
            Self::BogackiShampine23 => 1. / 3.,
        }
    }
    fn evaluations(self) -> usize {
        match self {
            Self::DormandPrince45 => 6,
            Self::BogackiShampine23 => 3,
        }
    }
    fn shrink(self) -> f64 {
        // Lower bound on the first retry's step ratio; subsequent retries halve.
        // Shampine & Reichelt (1997), step control for the low-order pair.
        match self {
            Self::DormandPrince45 => 0.1,
            Self::BogackiShampine23 => 0.5,
        }
    }
}

/// The derivative function f(t, y), supplied by name or handle;
/// `Engine::ode_derivative` is the one place that evaluates it.
enum OdeFunction {
    File(String),
    Handle(FunctionHandle),
}
impl OdeFunction {
    /// MATLAB returns the solution structure for `sol = odeXX(...)` only when f is a
    /// function handle; with an ODE-file name the one output is t.
    fn solution_struct(&self) -> bool {
        matches!(self, Self::Handle(_))
    }
    fn diagnostic_name(&self) -> String {
        match self {
            Self::File(name) => name.clone(),
            // MATLAB's ODE argument diagnostics uppercase the handle text.
            Self::Handle(handle) => handle.func2str().to_uppercase(),
        }
    }
}

struct OdeProblem {
    method: OdeMethod,
    tspan: Vec<f64>,
    /// 0-based index of the next requested output time in `tspan`.
    next: usize,
    t0: f64,
    tfinal: f64,
    htspan: f64,
    y0: Vec<f64>,
    rtol: f64,
    threshold: Vec<f64>,
    hmax: f64,
    htry: Option<f64>,
    refine: usize,
    /// Keep each step's stages for sol.idata.f3d (deval, odextend).
    keep_stages: bool,
}

struct OdeOutput {
    t: Vec<f64>,
    /// Point-major: one block of `neq` values per output time.
    y: Vec<f64>,
    /// Time and smallest allowed step when tolerances could not be met.
    failure: Option<(f64, f64)>,
    /// Accepted steps, failed attempts and derivative evaluations: MATLAB's sol.stats.
    steps: usize,
    failed: usize,
    evals: usize,
    /// Stages of each accepted step (sol output only).
    stages: Vec<Vec<Vec<f64>>>,
    stages_dropped: bool,
}
impl OdeOutput {
    fn push(&mut self, t: f64, y: &[f64]) -> Result<()> {
        if (self.t.len() + 1).saturating_mul(y.len().max(1)) > MAX_ELEMENTS {
            return Err(format!(
                "ODE solution exceeds the {MAX_ELEMENTS}-element limit; shorten tspan or request fewer output times"
            ));
        }
        self.t.push(t);
        self.y.extend_from_slice(y);
        Ok(())
    }
}

fn ode_eps(x: f64) -> f64 {
    // Binary64 spacing in the binade containing |x|, including subnormals.
    let exponent = ((x.to_bits() >> 52) & 0x7ff) as i32;
    match exponent {
        0 => f64::from_bits(1),
        1..=52 => f64::from_bits(1 << (exponent - 1)),
        2047 => f64::NAN,
        _ => f64::from_bits(((exponent - 52) as u64) << 52),
    }
}

fn ode_inf_norm(values: impl Iterator<Item = f64>) -> f64 {
    // Like norm(v, Inf), any NaN component makes the norm NaN.
    let mut norm: f64 = 0.;
    for value in values {
        if value.is_nan() {
            return f64::NAN;
        }
        norm = norm.max(value.abs());
    }
    norm
}

fn ode_error_norm(e: &[f64], y: &[f64], ynew: &[f64], threshold: &[f64]) -> f64 {
    ode_inf_norm(
        e.iter()
            .enumerate()
            .map(|(i, error)| error / y[i].abs().max(ynew[i].abs()).max(threshold[i])),
    )
}

/// One accepted or rejected attempt: the stages needed for error control and output.
struct OdeAttempt {
    tnew: f64,
    h: f64,
    ynew: Vec<f64>,
    err: f64,
    /// ode45: f1, f3, f4, f5, f6, f7; ode23: f1, f2, f3, f4. The last is f(tnew, ynew).
    stages: Vec<Vec<f64>>,
}

type Derivative<'a> = dyn FnMut(f64, &[f64]) -> Result<Vec<f64>> + 'a;

fn ode_combine(y: &[f64], h: f64, terms: &[(f64, &[f64])]) -> Vec<f64> {
    (0..y.len())
        .map(|i| {
            let increment = terms
                .iter()
                .fold(0., |sum, (weight, stage)| sum + weight * stage[i]);
            y[i] + h * increment
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn ode_attempt(
    p: &OdeProblem,
    t: f64,
    y: &[f64],
    mut h: f64,
    absh: f64,
    done: bool,
    f1: &[f64],
    f: &mut Derivative,
) -> Result<OdeAttempt> {
    let tnew = if done { p.tfinal } else { t + h };
    // Use the representable interval for the final, endpoint-clipped step.
    if done {
        h = tnew - t;
    }
    let (ynew, error, stages) = match p.method {
        OdeMethod::DormandPrince45 => {
            // Dormand & Prince (1980), Table 2: the seven-stage RK5(4)7M pair.
            let k2 = f(t + h / 5., &ode_combine(y, h, &[(1. / 5., f1)]))?;
            let k3 = f(
                t + 3. * h / 10.,
                &ode_combine(y, h, &[(3. / 40., f1), (9. / 40., &k2)]),
            )?;
            let k4 = f(
                t + 4. * h / 5.,
                &ode_combine(y, h, &[(44. / 45., f1), (-56. / 15., &k2), (32. / 9., &k3)]),
            )?;
            let k5 = f(
                t + 8. * h / 9.,
                &ode_combine(
                    y,
                    h,
                    &[
                        (19372. / 6561., f1),
                        (-25360. / 2187., &k2),
                        (64448. / 6561., &k3),
                        (-212. / 729., &k4),
                    ],
                ),
            )?;
            let k6 = f(
                tnew,
                &ode_combine(
                    y,
                    h,
                    &[
                        (9017. / 3168., f1),
                        (-355. / 33., &k2),
                        (46732. / 5247., &k3),
                        (49. / 176., &k4),
                        (-5103. / 18656., &k5),
                    ],
                ),
            )?;
            let next = ode_combine(
                y,
                h,
                &[
                    (35. / 384., f1),
                    (500. / 1113., &k3),
                    (125. / 192., &k4),
                    (-2187. / 6784., &k5),
                    (11. / 84., &k6),
                ],
            );
            let k7 = f(tnew, &next)?;
            // Difference of the fifth- and fourth-order weights, before h scaling.
            let error = ode_combine(
                &vec![0.; y.len()],
                1.,
                &[
                    (71. / 57600., f1),
                    (-71. / 16695., &k3),
                    (71. / 1920., &k4),
                    (-17253. / 339200., &k5),
                    (22. / 525., &k6),
                    (-1. / 40., &k7),
                ],
            );
            (next, error, vec![f1.to_vec(), k3, k4, k5, k6, k7])
        }
        OdeMethod::BogackiShampine23 => {
            // Bogacki & Shampine (1989), the four-stage 3(2) pair.
            let k2 = f(t + h / 2., &ode_combine(y, h, &[(0.5, f1)]))?;
            let k3 = f(t + 3. * h / 4., &ode_combine(y, h, &[(0.75, &k2)]))?;
            let next = ode_combine(y, h, &[(2. / 9., f1), (1. / 3., &k2), (4. / 9., &k3)]);
            let k4 = f(tnew, &next)?;
            let error = ode_combine(
                &vec![0.; y.len()],
                1.,
                &[
                    (5. / 72., f1),
                    (-1. / 12., &k2),
                    (-1. / 9., &k3),
                    (1. / 8., &k4),
                ],
            );
            (next, error, vec![f1.to_vec(), k2, k3, k4])
        }
    };
    let err = absh * ode_error_norm(&error, y, &ynew, &p.threshold);
    Ok(OdeAttempt {
        tnew,
        h,
        ynew,
        err,
        stages,
    })
}

fn ode_interpolate(method: OdeMethod, tint: f64, t: f64, y: &[f64], step: &OdeAttempt) -> Vec<f64> {
    if tint == t {
        return y.to_vec();
    }
    if tint == step.tnew {
        return step.ynew.clone();
    }
    let s = (tint - t) / step.h;
    match method {
        OdeMethod::DormandPrince45 => {
            // Shampine (1986), section 3, (3.1)-(3.5), Case I: the free
            // fourth-order extension of the Dormand-Prince tableau. The
            // quartic coefficient of k7 is 5/2, selected by the public output
            // fixtures. All remaining coefficients follow from those order
            // conditions and the endpoint values/slopes; exact derivation:
            // tool/cleanroom-ode45/shampine1986.py. Stage k2 has zero weight.
            let weights = [
                s * (1. + s * (-183. / 64. + s * (37. / 12. - s * 145. / 128.))),
                s * s * (1500. / 371. + s * (-1000. / 159. + s * 1000. / 371.)),
                s * s * (-125. / 32. + s * (125. / 12. - s * 375. / 64.)),
                s * s * (9477. / 3392. + s * (-729. / 106. + s * 25515. / 6784.)),
                s * s * (-11. / 7. + s * (11. / 3. - s * 55. / 28.)),
                s * s * (3. / 2. + s * (-4. + s * 5. / 2.)),
            ];
            (0..y.len())
                .map(|i| {
                    let increment = weights
                        .iter()
                        .zip(&step.stages)
                        .fold(0., |sum, (weight, stage)| sum + weight * stage[i]);
                    y[i] + step.h * increment
                })
                .collect()
        }
        OdeMethod::BogackiShampine23 => {
            // Cubic Hermite: the endpoint values and slopes uniquely determine
            // these coefficients. Shampine (1986), section 3, p.144.
            let complement = 1. - s;
            y.iter()
                .enumerate()
                .map(|(i, &start)| {
                    let delta = step.ynew[i] - start;
                    let initial = step.h * step.stages[0][i] - delta;
                    let curvature = 2. * delta
                        - step.h * step.stages[0][i]
                        - step.h * step.stages.last().unwrap()[i];
                    start + s * (delta + complement * (initial + s * curvature))
                })
                .collect()
        }
    }
}

fn ode_integrate(p: &OdeProblem, f0: Vec<f64>, f: &mut Derivative) -> Result<OdeOutput> {
    let mut out = OdeOutput {
        t: Vec::new(),
        y: Vec::new(),
        failure: None,
        steps: 0,
        failed: 0,
        evals: 1,
        stages: Vec::new(),
        stages_dropped: false,
    };
    out.push(p.t0, &p.y0)?;
    let direction = (p.tfinal - p.t0).signum();
    let mut t = p.t0;
    let mut y = p.y0.clone();
    let mut first = f0;
    let mut next = p.next;
    let mut absh = p.htry.unwrap_or_else(|| {
        // Scale the initial slope to the local-error target (HN&W, II.4).
        let slope = ode_inf_norm(
            first
                .iter()
                .zip(&y)
                .zip(&p.threshold)
                .map(|((&dy, &yi), &floor)| dy / yi.abs().max(floor)),
        );
        let rate = slope / (0.8 * libm::pow(p.rtol, p.method.pow()));
        let trial = p.hmax.min(p.htspan);
        if trial * rate > 1. { 1. / rate } else { trial }
    });
    loop {
        let hmin = 16. * ode_eps(t);
        // MaxStep remains the cap even when it is below the minimum retry step.
        absh = absh.max(hmin).min(p.hmax);
        let mut retried = false;
        let accepted = loop {
            // Only a fresh step may stretch by ten percent to reach the end.
            // A retry must keep its reduced length, including at the hmin floor.
            let done = !retried && 1.1 * absh >= (p.tfinal - t).abs();
            let h = if done { p.tfinal - t } else { direction * absh };
            let step = ode_attempt(p, t, &y, h, h.abs(), done, &first, f)?;
            out.evals += p.method.evaluations();
            // Reject only err > rtol: an unordered (NaN) comparison accepts.
            if step.err <= p.rtol || step.err.is_nan() || p.rtol.is_nan() {
                break step;
            }
            out.failed += 1;
            // A clipped final attempt may already be shorter than absh.
            // Stop before the minimum retry length could overshoot tfinal.
            if absh.min(h.abs()) <= hmin {
                out.failure = Some((t, hmin));
                return Ok(out);
            }
            let factor = if retried {
                0.5
            } else {
                (0.8 * libm::pow(p.rtol / step.err, p.method.pow())).max(p.method.shrink())
            };
            absh = (h.abs() * factor).max(hmin);
            retried = true;
        };
        out.steps += 1;
        if p.tspan.len() > 2 {
            while next < p.tspan.len() && direction * (p.tspan[next] - accepted.tnew) <= 0. {
                let ti = p.tspan[next];
                out.push(ti, &ode_interpolate(p.method, ti, t, &y, &accepted))?;
                next += 1;
            }
        } else {
            for j in 1..=p.refine {
                let ti = if j == p.refine {
                    accepted.tnew
                } else {
                    t + accepted.h * (j as f64 / p.refine as f64)
                };
                out.push(ti, &ode_interpolate(p.method, ti, t, &y, &accepted))?;
            }
        }
        if !retried {
            let ratio = libm::pow(accepted.err / p.rtol, p.method.pow()) / 0.8;
            absh = accepted.h.abs() * if ratio > 0.2 { 1. / ratio } else { 5. };
        } else {
            absh = accepted.h.abs();
        }
        t = accepted.tnew;
        first = accepted.stages.last().unwrap().clone();
        y = accepted.ynew;
        if p.keep_stages && !out.stages_dropped {
            // Include the leading zero block inserted by ode_solution.
            let stored = (out.steps + 1)
                .saturating_mul(y.len())
                .saturating_mul(accepted.stages.len());
            if stored > MAX_ELEMENTS {
                out.stages.clear();
                out.stages_dropped = true;
            } else {
                out.stages.push(accepted.stages);
            }
        }
        if t == p.tfinal {
            return Ok(out);
        }
    }
}

/// C printf "%e" formatting, as MATLAB's sprintf('%e', x).
fn ode_format_e(x: f64) -> String {
    if !x.is_finite() {
        return format!("{x}");
    }
    let text = format!("{x:.6e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.abs())
}

/// Canonical odeset name for a case-insensitive prefix, as MATLAB's odeset/odeget.
fn ode_property(name: &str) -> Result<&'static str> {
    let lower = name.to_lowercase();
    let matches: Vec<&str> = ODE_NAMES
        .iter()
        .copied()
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .collect();
    match matches[..] {
        [] => Err(format!("Unrecognized property name '{name}'.")),
        [only] => Ok(only),
        _ => match matches.iter().find(|n| n.to_lowercase() == lower) {
            Some(exact) => Ok(exact),
            None => Err(format!(
                "Ambiguous property name '{name}' ({}).",
                matches.join(", ")
            )),
        },
    }
}

/// Values of an odeset option set: supported names with non-empty values.
type OdeOptions = Vec<(&'static str, Arc<Array>)>;

impl Engine {
    /// Single-output calls: odeset, odeget and t = ode45(...).
    pub(super) fn ode_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        Ok(Some(match name {
            "odeset" => self.ode_set(args)?,
            "odeget" => self.ode_get(args)?,
            // eval charges this result; ode_solve would charge it a second time.
            _ if is_solver(name) => self.ode_outputs(name, args.to_vec(), 1, out)?.remove(0),
            _ => return Ok(None),
        }))
    }
    /// [t, y] = ode45(f, tspan, y0, options) and ode23 from eval_outputs, which does not
    /// charge results against the statement budget.
    pub(super) fn ode_solve(
        &mut self,
        name: &str,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let outputs = self.ode_outputs(name, args, count, out)?;
        self.charge_function_outputs(&outputs)?;
        Ok(outputs)
    }
    /// Solves and builds the outputs without charging them.
    fn ode_outputs(
        &mut self,
        name: &str,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let method = if name == "ode45" {
            OdeMethod::DormandPrince45
        } else {
            OdeMethod::BogackiShampine23
        };
        if count > 2 {
            return Err(format!(
                "{name} supports at most two outputs [t, y]; event outputs (te, ye, ie) are not supported yet"
            ));
        }
        if args.is_empty() {
            return Err(format!(
                "Not enough input arguments. See {}.",
                name.to_uppercase()
            ));
        }
        if args.len() > 4 {
            return Err(format!(
                "{name} extra parameters after options are not supported; pass them through a function instead"
            ));
        }
        let args = self.text_arguments(args)?;
        let function = match &args[0] {
            Value::Text(text) => OdeFunction::File(text.clone()),
            Value::Container(Array::Handle(handle)) => OdeFunction::Handle(handle.clone()),
            _ => {
                return Err(format!(
                    "{name} expects a derivative function handle or function name as text"
                ));
            }
        };
        if args.len() < 3 || self.ode_is_empty(&args[1]) || self.ode_is_empty(&args[2]) {
            if matches!(function, OdeFunction::Handle(_)) {
                return Err(format!(
                    "When the first argument to {name} is a function handle, the tspan and y0 arguments must be supplied."
                ));
            }
            return Err(format!(
                "{name} requires non-empty tspan and y0; reading defaults from the ODE file is not supported"
            ));
        }
        let tspan = self.ode_double(&args[1])?;
        if matches!(args[2], Value::Logical(_)) {
            return Err("Invalid type for argument Y. Type should be double.".into());
        }
        let y0 = self.ode_double(&args[2])?;
        let options = match args.get(3) {
            None => Vec::new(),
            Some(value) => self.ode_options(value, name)?,
        };
        let option = |key: &str| options.iter().find(|(n, _)| *n == key).map(|(_, v)| v);
        // Function-name input keeps MATLAB's ODE-file form tspan = tf (from 0).
        let (t0, next) = if tspan.len() == 1 {
            (0., 0)
        } else {
            (tspan[0], 1)
        };
        let htspan = (tspan[next] - t0).abs();
        let tfinal = tspan[tspan.len() - 1];
        // The reference does not check (NaN reaches its plotting in transcript call2); Inf would step without end.
        if tspan.iter().any(|t| !t.is_finite()) {
            return Err("The entries in tspan must be finite.".into());
        }
        if t0 == tfinal {
            return Err("The last entry in tspan must be different from the first entry.".into());
        }
        let tdir = (tfinal - t0).signum();
        if tspan.windows(2).any(|w| tdir * (w[1] - w[0]) <= 0.) {
            return Err("The entries in tspan must strictly increase or decrease.".into());
        }
        let neq = y0.len();
        let f0 = self.ode_derivative(&function, t0, &y0, out)?;
        let rtol = match option("RelTol") {
            None => 1e-3,
            Some(v) => match &**v {
                // NaN passes, as MATLAB tests only rtol <= 0.
                Array::Numeric(m) if m.len() == 1 && (m.data()[0] > 0. || m.data()[0].is_nan()) => {
                    m.data()[0]
                }
                _ => return Err("RelTol must be a positive scalar.".into()),
            },
        };
        let rtol = if rtol < 100. * f64::EPSILON {
            // MATLAB prints 100*eps with %g.
            self.emit(
                out,
                "Warning: RelTol has been increased to 2.22045e-14.".into(),
            )?;
            100. * f64::EPSILON
        } else {
            rtol
        };
        let atol = match option("AbsTol") {
            None => vec![1e-6],
            Some(v) => match &**v {
                Array::Numeric(m) => m.data().to_vec(),
                _ => return Err("AbsTol must be positive.".into()),
            },
        };
        if atol.iter().any(|a| *a <= 0.) {
            return Err("AbsTol must be positive.".into());
        }
        if atol.len() != 1 && atol.len() != neq {
            let f = function.diagnostic_name();
            return Err(format!(
                "Solving {f} requires a scalar AbsTol, or a vector AbsTol of length {neq}"
            ));
        }
        let threshold = (0..neq)
            .map(|i| atol[if atol.len() == 1 { 0 } else { i }] / rtol)
            .collect();
        let max_step = match option("MaxStep") {
            None => 0.1 * (tfinal - t0),
            Some(v) => self.ode_option_scalar(v, "MaxStep")?,
        };
        let hmax = (tfinal - t0).abs().min(max_step.abs());
        if hmax <= 0. {
            return Err("Option 'MaxStep' must be greater than zero.".into());
        }
        let htry = match option("InitialStep") {
            None => None,
            Some(v) => Some(self.ode_option_scalar(v, "InitialStep")?.abs()),
        };
        if htry.is_some_and(|h| h <= 0.) {
            return Err("Option 'InitialStep' must be greater than zero.".into());
        }
        let refine = match option("Refine") {
            None if method == OdeMethod::DormandPrince45 => 4.,
            None => 1.,
            Some(v) => self.ode_option_scalar(v, "Refine")?.max(1.),
        };
        if refine.fract() != 0. || refine > 1e6 {
            return Err("Refine must be a positive integer of at most 1000000".into());
        }
        // sol holds the solver steps: no refinement, no interior output times.
        let solution_output = count == 1 && function.solution_struct();
        let (tspan, next, refine) = if solution_output {
            (vec![t0, tfinal], 1, 1.)
        } else {
            (tspan, next, refine)
        };
        let problem = OdeProblem {
            method,
            tspan,
            next,
            t0,
            tfinal,
            htspan,
            y0,
            rtol,
            threshold,
            hmax,
            htry,
            refine: refine as usize,
            keep_stages: solution_output,
        };
        let solution = {
            let mut derivative = |t: f64, y: &[f64]| -> Result<Vec<f64>> {
                self.ode_derivative(&function, t, y, out)
            };
            ode_integrate(&problem, f0, &mut derivative)?
        };
        if let Some((t, hmin)) = solution.failure {
            self.emit(
                out,
                format!(
                    "Warning: Integration stopped at t={}; meeting the tolerances would require a step below the minimum ({}).",
                    ode_format_e(t),
                    ode_format_e(hmin)
                ),
            )?;
        }
        self.check_cancel()?;
        if solution_output {
            let extdata = self.ode_extdata(&function, args.get(3))?;
            return Ok(vec![self.ode_solution(name, &solution, neq, extdata)?]);
        }
        let points = solution.t.len();
        let t = Matrix::from_column_vec(points, 1, solution.t)?;
        let mut outputs = vec![Value::Matrix(t)];
        if count == 2 {
            let mut y = Vec::with_capacity(points * neq);
            for j in 0..neq {
                y.extend((0..points).map(|i| solution.y[i * neq + j]));
            }
            outputs.push(Value::Matrix(Matrix::from_column_vec(points, neq, y)?));
        }
        Ok(outputs)
    }
    /// Evaluates f(t, y) and checks it returns a column of length numel(y).
    fn ode_derivative(
        &mut self,
        function: &OdeFunction,
        t: f64,
        y: &[f64],
        out: &mut Execution,
    ) -> Result<Vec<f64>> {
        self.check_cancel()?;
        let args = vec![
            Value::Matrix(Matrix::scalar(t)),
            Value::Matrix(Matrix::from_column_slice(y.len(), 1, y)?),
        ];
        // Derivatives are transient solver stages, not retained statement outputs.
        // Keep all limits inside the call, then release its charges on both success
        // and error. ode_solve / eval charge the stored solution when it is returned.
        let evaluated = self.evaluated_elements;
        let result = (|| match function {
            OdeFunction::File(name) => {
                let Some(resolved) = self.resolve_function(name)? else {
                    return Err(format!(
                        "Undefined function '{name}': the ODE function must be a .m/.mr function file in the Current Folder or a local function"
                    ));
                };
                self.invoke_function(resolved, args, 1, None, out)
            }
            OdeFunction::Handle(handle) => self.call_handle(handle, args, 1, None, out),
        })();
        self.evaluated_elements = evaluated;
        let values = result?;
        let value = values
            .into_iter()
            .next()
            .ok_or_else(|| {
                format!(
                    "{} must return a derivative column vector.",
                    function.diagnostic_name()
                )
            })?
            .matrix(&self.cancel)?;
        if value.cols() > 1 {
            return Err(format!(
                "{} must return a column vector.",
                function.diagnostic_name()
            ));
        }
        if value.rows() != y.len() || value.cols() == 0 {
            let name = function.diagnostic_name();
            return Err(format!(
                "{name} returns a vector of length {}, but the length of initial conditions vector is {}. The vector returned by {name} and the initial conditions vector must have the same number of elements.",
                value.len(),
                y.len()
            ));
        }
        Ok(value.data().to_vec())
    }
    fn ode_is_empty(&self, value: &Value) -> bool {
        match value {
            Value::Matrix(m) => m.is_empty(),
            Value::Logical(m) => m.is_empty(),
            Value::Complex(m) => m.is_empty(),
            Value::Container(a) => a.is_empty(),
            Value::Text(s) | Value::QuotedString(s) => s.is_empty(),
            Value::Void => true,
        }
    }
    /// tspan and y0: real double arrays, read in column-major order like x(:).
    fn ode_double(&self, value: &Value) -> Result<Vec<f64>> {
        match value {
            Value::Matrix(m) => Ok(m.data().to_vec()),
            Value::Complex(_) => Err(
                "Complex tspan or y0 is not supported yet; split into real and imaginary parts"
                    .into(),
            ),
            _ => Err("Inputs must be floats, namely single or double.".into()),
        }
    }
    fn ode_option_scalar(&self, value: &Array, name: &str) -> Result<f64> {
        match value {
            Array::Numeric(m) if m.len() == 1 => Ok(m.data()[0]),
            _ => Err(format!("{name} must be a real numeric scalar")),
        }
    }
    /// Reads an option set made by odeset; [] means defaults. Returns the non-empty
    /// settings and rejects any that these solvers do not implement.
    fn ode_options(&self, value: &Value, caller: &str) -> Result<OdeOptions> {
        if self.ode_is_empty(value) {
            return Ok(Vec::new());
        }
        let Value::Container(Array::Struct(set)) = value else {
            return Err(format!(
                "{caller} options must be [] or an option set created with odeset"
            ));
        };
        let mut options = Vec::new();
        for name in ODE_NAMES {
            let Some(field) = set.field_index(name) else {
                continue;
            };
            let setting = set.get(field, 0)?;
            if setting.is_empty() {
                continue;
            }
            if !ODE_SUPPORTED.contains(&name) {
                return Err(format!(
                    "{caller} option '{name}' is not supported yet; supported: {}",
                    ODE_SUPPORTED.join(", ")
                ));
            }
            options.push((name, setting.clone()));
        }
        Ok(options)
    }
    /// odeset(name, value, ...), odeset(oldopts, name, value, ...), odeset(oldopts, newopts):
    /// a struct with every odeset field in MATLAB's order, [] where unset.
    fn ode_set(&self, args: &[Value]) -> Result<Value> {
        let args = self.text_arguments(args.to_vec())?;
        let unset = Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?));
        let mut slots = vec![unset; ODE_NAMES.len()];
        let mut i = 0;
        while i < args.len() && !matches!(args[i], Value::Text(_)) {
            match &args[i] {
                value if self.ode_is_empty(value) => {}
                Value::Container(Array::Struct(set)) if set.len() == 1 => {
                    for (slot, name) in slots.iter_mut().zip(ODE_NAMES) {
                        let Some(field) = set.field_index(name) else {
                            continue;
                        };
                        let setting = set.get(field, 0)?;
                        if !setting.is_empty() {
                            *slot = setting.clone();
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "Expected argument {} to be a string property name or an options structure created with ODESET.",
                        i + 1
                    ));
                }
            }
            i += 1;
        }
        if !(args.len() - i).is_multiple_of(2) {
            return Err("Arguments must occur in name-value pairs.".into());
        }
        while i < args.len() {
            let Value::Text(name) = &args[i] else {
                return Err(format!(
                    "Expected argument {} to be a string property name.",
                    i + 1
                ));
            };
            let property = ode_property(name)?;
            if let Some(slot) = ODE_NAMES
                .iter()
                .position(|n| *n == property)
                .and_then(|at| slots.get_mut(at))
            {
                *slot = Arc::new(args[i + 1].clone().array(&self.cancel)?);
            }
            i += 2;
        }
        let names = ODE_NAMES.iter().map(|n| (*n).to_string()).collect();
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?)))
    }
    /// odeget(options, name) and odeget(options, name, default).
    fn ode_get(&self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let args = self.text_arguments(args.to_vec())?;
        let default = args
            .get(2)
            .cloned()
            .unwrap_or(Value::Matrix(Matrix::filled(0, 0, 0.)?));
        if self.ode_is_empty(&args[0]) {
            return Ok(default);
        }
        let Value::Container(Array::Struct(set)) = &args[0] else {
            return Err("First argument must be an options structure created with ODESET.".into());
        };
        let Value::Text(name) = &args[1] else {
            return Err("Property name must be a character vector.".into());
        };
        let property = ode_property(name).map_err(|e| {
            if e.starts_with("Unrecognized") {
                format!("{e}  See ODESET for possibilities.")
            } else {
                e
            }
        })?;
        let setting = match set.field_index(property) {
            Some(field) if set.len() == 1 => Some(set.get(field, 0)?),
            _ => None,
        };
        Ok(match setting {
            Some(value) if !value.is_empty() => Value::from((**value).clone()),
            _ => default,
        })
    }
    /// sol.extdata: the ODE function, the options as given and the extra arguments.
    fn ode_extdata(&self, function: &OdeFunction, options: Option<&Value>) -> Result<Array> {
        let odefun = match function {
            OdeFunction::Handle(h) => Array::Handle(h.clone()),
            OdeFunction::File(n) => Array::Handle(FunctionHandle::named(n)),
        };
        let options = match options {
            Some(v) => v.clone().array(&self.cancel)?,
            None => Array::Numeric(Matrix::filled(0, 0, 0.)?),
        };
        let varargin = Array::Cell(crate::CellArray::empty(1, 0, &self.cancel)?);
        Ok(Array::Struct(StructArray::from_slots(
            1,
            1,
            ["odefun", "options", "varargin"].map(String::from).to_vec(),
            vec![Arc::new(odefun), Arc::new(options), Arc::new(varargin)],
            &self.cancel,
        )?))
    }
    /// sol = odeXX(f, ...): solver name, extdata, solver steps x (1-by-N), y (neq-by-N),
    /// stats and idata.f3d (each step's stages, neq-by-(stages*N), zeros for the first point).
    fn ode_solution(
        &self,
        name: &str,
        solution: &OdeOutput,
        neq: usize,
        extdata: Array,
    ) -> Result<Value> {
        let points = solution.t.len();
        let scalar = |n: usize| Arc::new(Array::Numeric(Matrix::scalar(n as f64)));
        let stats = StructArray::from_slots(
            1,
            1,
            ["nsteps", "nfailed", "nfevals"].map(String::from).to_vec(),
            vec![
                scalar(solution.steps),
                scalar(solution.failed),
                scalar(solution.evals),
            ],
            &self.cancel,
        )?;
        let x = Matrix::from_column_vec(1, points, solution.t.clone())?;
        let y = Matrix::from_column_vec(neq, points, solution.y.clone())?;
        let solver = CharacterRow::from_text(name, &self.cancel)?;
        let width = solution.stages.first().map_or(0, Vec::len);
        let mut f3d = if solution.stages.is_empty() {
            Vec::new()
        } else {
            vec![0.; neq * width]
        };
        for step in &solution.stages {
            for stage in step {
                f3d.extend_from_slice(stage);
            }
        }
        let f3d = Matrix::from_column_vec(neq, f3d.len() / neq.max(1), f3d)?;
        let idata = StructArray::from_slots(
            1,
            1,
            ["f3d", "idxNonNegative"].map(String::from).to_vec(),
            vec![
                Arc::new(Array::Numeric(f3d)),
                Arc::new(Array::Numeric(Matrix::filled(0, 0, 0.)?)),
            ],
            &self.cancel,
        )?;
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            ["solver", "extdata", "x", "y", "stats", "idata"]
                .map(String::from)
                .to_vec(),
            vec![
                Arc::new(Array::Character(solver)),
                Arc::new(extdata),
                Arc::new(Array::Numeric(x)),
                Arc::new(Array::Numeric(y)),
                Arc::new(Array::Struct(stats)),
                Arc::new(Array::Struct(idata)),
            ],
            &self.cancel,
        )?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transient_derivative_charges_are_released_even_on_error() {
        let mut e = Engine::default();
        e.execute("f=@(t,y) -y; bad=@(t,y) ones(100,1);").unwrap();
        let f = OdeFunction::Handle(e.handle_variable("f").unwrap());
        let bad = OdeFunction::Handle(e.handle_variable("bad").unwrap());
        let charged = 32 * 1024 * 1024 - 32;
        e.evaluated_elements = charged;
        let mut out = Execution::default();
        for _ in 0..100 {
            assert_eq!(
                e.ode_derivative(&f, 0., &[1., 2.], &mut out).unwrap(),
                [-1., -2.]
            );
            assert_eq!(e.evaluated_elements, charged);
        }
        assert!(
            e.ode_derivative(&bad, 0., &[1., 2.], &mut out)
                .unwrap_err()
                .contains("budget")
        );
        assert_eq!(e.evaluated_elements, charged);
        e.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            e.ode_derivative(&f, 0., &[1., 2.], &mut out)
                .unwrap_err()
                .contains("cancelled")
        );
        assert_eq!(e.evaluated_elements, charged);
    }

    #[test]
    fn stored_solver_outputs_still_consume_the_statement_budget() {
        let mut e = Engine::default();
        e.execute("f=@(t,y) -y;").unwrap();
        let args = vec![
            Value::Container(Array::Handle(e.handle_variable("f").unwrap())),
            Value::Matrix(Matrix::from_column_vec(1, 2, vec![0., 1.]).unwrap()),
            Value::Matrix(Matrix::scalar(1.)),
        ];
        e.evaluated_elements = 32 * 1024 * 1024 - 32;
        assert!(
            e.ode_solve("ode45", args, 2, &mut Execution::default())
                .unwrap_err()
                .contains("budget")
        );
    }
    #[test]
    fn eps_matches_matlab_spacing() {
        assert_eq!(ode_eps(1.), f64::EPSILON);
        assert_eq!(ode_eps(0.), f64::from_bits(1));
        assert_eq!(ode_eps(-2.), 2. * f64::EPSILON);
        assert_eq!(ode_eps(20.), 16. * f64::EPSILON);
        assert_eq!(ode_eps(f64::MIN_POSITIVE), f64::from_bits(1));
    }
    #[test]
    fn printf_exponent_format() {
        assert_eq!(ode_format_e(0.5), "5.000000e-01");
        assert_eq!(ode_format_e(1234.5), "1.234500e+03");
        assert_eq!(ode_format_e(0.), "0.000000e+00");
    }
    #[test]
    fn solution_structure_has_solver_x_y_and_stats() {
        let output = OdeOutput {
            t: vec![0., 0.5, 1.],
            y: vec![1., 2., 3., 4., 5., 6.],
            failure: None,
            steps: 2,
            failed: 1,
            evals: 19,
            stages: Vec::new(),
            stages_dropped: false,
        };
        let e = Engine::default();
        let extdata = Array::Numeric(Matrix::filled(0, 0, 0.).unwrap());
        let value = e.ode_solution("ode45", &output, 2, extdata).unwrap();
        let Value::Container(Array::Struct(sol)) = value else {
            panic!("sol is a struct");
        };
        let names: Vec<&str> = sol.field_names().iter().map(String::as_str).collect();
        assert_eq!(names, ["solver", "extdata", "x", "y", "stats", "idata"]);
        let field = |name: &str| Arc::clone(sol.get(sol.field_index(name).unwrap(), 0).unwrap());
        let (Array::Numeric(x), Array::Numeric(y), Array::Struct(stats)) =
            (&*field("x"), &*field("y"), &*field("stats"))
        else {
            panic!("x and y are numeric, stats a struct");
        };
        // x is 1-by-N; y is neq-by-N with one column per time.
        assert_eq!((x.rows(), x.cols(), x.data()), (1, 3, &[0., 0.5, 1.][..]));
        assert_eq!((y.rows(), y.cols()), (2, 3));
        assert_eq!((y.get(0, 1), y.get(1, 2)), (3., 6.));
        let names: Vec<&str> = stats.field_names().iter().map(String::as_str).collect();
        assert_eq!(names, ["nsteps", "nfailed", "nfevals"]);
    }
    #[test]
    fn property_prefixes_resolve_like_odeset() {
        assert_eq!(ode_property("reltol").unwrap(), "RelTol");
        assert_eq!(
            ode_property("Max").unwrap_err(),
            "Ambiguous property name 'Max' (MaxOrder, MaxStep)."
        );
        assert_eq!(ode_property("mass").unwrap(), "Mass");
        assert!(ode_property("Foo").unwrap_err().contains("Unrecognized"));
    }
}

// These tests also run in tool/cleanroom-ode45 against the extracted core.
#[cfg(test)]
mod numerical_tests {
    use super::*;

    fn problem(method: OdeMethod, tspan: Vec<f64>, y0: Vec<f64>) -> OdeProblem {
        let t0 = tspan[0];
        let tfinal = *tspan.last().unwrap();
        let neq = y0.len();
        OdeProblem {
            method,
            htspan: (tspan[1] - t0).abs(),
            tspan,
            next: 1,
            t0,
            tfinal,
            y0,
            rtol: 1e-8,
            threshold: vec![1e-2; neq],
            hmax: (tfinal - t0).abs() / 10.,
            htry: None,
            refine: 4,
            keep_stages: true,
        }
    }

    #[test]
    fn binary_spacing_across_binades() {
        assert_eq!(ode_eps(0.), f64::from_bits(1));
        for exponent in 1..2047u64 {
            let x = f64::from_bits(exponent << 52);
            assert_eq!(ode_eps(x), f64::from_bits(x.to_bits() + 1) - x);
            assert_eq!(ode_eps(-x), ode_eps(x));
        }
        assert!(ode_eps(f64::INFINITY).is_nan());
    }

    #[test]
    fn exact_decay_dense_output_and_fsal_accounting() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            for times in [vec![0., 1.], vec![1., 0.], vec![0., 0.13, 0.4, 0.91, 1.]] {
                let p = problem(method, times, vec![1., 2.]);
                let mut calls = 1;
                let out = ode_integrate(&p, vec![-1., -2.], &mut |_, y| {
                    calls += 1;
                    Ok(y.iter().map(|v| -v).collect())
                })
                .unwrap();
                assert_eq!(out.evals, calls);
                assert_eq!(
                    out.evals,
                    1 + (out.steps + out.failed) * method.evaluations()
                );
                assert_eq!(out.stages.len(), out.steps);
                assert!(out.failure.is_none());
                for (i, &t) in out.t.iter().enumerate() {
                    for j in 0..2 {
                        let exact = (j + 1) as f64 * libm::exp(p.t0 - t);
                        assert!((out.y[2 * i + j] - exact).abs() < 1e-7);
                    }
                }
                assert_eq!(*out.t.last().unwrap(), p.tfinal);
                if p.tspan.len() > 2 {
                    assert_eq!(out.t, p.tspan);
                }
            }
        }
    }

    #[test]
    fn polynomial_dense_output_in_both_directions() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            for h in [-0.2, 0.2] {
                let p = problem(method, vec![0., h], vec![0.]);
                let s = ode_attempt(&p, 0., &[0.], h, h.abs(), true, &[0.], &mut |t, _| {
                    Ok(vec![3. * t * t])
                })
                .unwrap();
                for theta in [0., 0.1, 0.37, 0.9, 1.] {
                    let t = theta * h;
                    assert!(
                        (ode_interpolate(method, t, 0., &[0.], &s)[0] - t * t * t).abs() < 1e-15
                    );
                }
                let delta = 1e-5;
                let at = |theta| ode_interpolate(method, theta * h, 0., &[0.], &s)[0];
                assert!(((at(delta) - at(0.)) / (delta * h)).abs() < 1e-9);
                assert!(((at(1.) - at(1. - delta)) / (delta * h) - 3. * h * h).abs() < 3e-6);
            }
        }
    }

    #[test]
    fn embedded_pair_local_orders() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            let mut defects = Vec::new();
            let mut estimates = Vec::new();
            for h in [0.2, 0.1] {
                let p = problem(method, vec![0., h], vec![1.]);
                let s = ode_attempt(&p, 0., &[1.], h, h, true, &[1.], &mut |_, y| Ok(y.to_vec()))
                    .unwrap();
                defects.push((s.ynew[0] - libm::exp(h)).abs());
                estimates.push(s.err * s.ynew[0]);
            }
            let defect_ratio = defects[0] / defects[1];
            let estimate_ratio = estimates[0] / estimates[1];
            let (defect_range, estimate_range) = match method {
                OdeMethod::DormandPrince45 => (45. ..90., 25. ..40.),
                OdeMethod::BogackiShampine23 => (14. ..20., 6. ..12.),
            };
            assert!(defect_range.contains(&defect_ratio), "{defect_ratio}");
            assert!(estimate_range.contains(&estimate_ratio), "{estimate_ratio}");
        }
    }

    #[test]
    fn rejected_steps_preserve_fsal_and_propagate_cancellation() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            let mut p = problem(method, vec![0., 1.], vec![1.]);
            // Avoid z=-1, where BS23's embedded estimate for y'=lambda*y
            // vanishes even though the third-order solution has a defect.
            p.htry = Some(0.7);
            p.hmax = 1.;
            let mut calls = 1;
            let out = ode_integrate(&p, vec![-10.], &mut |_, y| {
                calls += 1;
                Ok(vec![-10. * y[0]])
            })
            .unwrap();
            assert!(out.failed > 0);
            assert_eq!(out.evals, calls);
            assert_eq!(
                out.evals,
                1 + (out.steps + out.failed) * method.evaluations()
            );
            assert!(out.failure.is_none(), "{:?}", out.failure);
            assert_eq!(*out.t.last().unwrap(), 1.);
            assert!(
                (out.y.last().unwrap() - libm::exp(-10.)).abs() < 1e-8,
                "last={} exact={} steps={} failed={}",
                out.y.last().unwrap(),
                libm::exp(-10.),
                out.steps,
                out.failed
            );
            let mut calls = 0;
            let error = ode_integrate(&p, vec![-1.], &mut |_, y| {
                calls += 1;
                if calls == 2 {
                    Err("cancelled".into())
                } else {
                    Ok(y.to_vec())
                }
            })
            .err()
            .unwrap();
            assert_eq!(error, "cancelled");
            assert_eq!(calls, 2);
        }
    }

    #[test]
    fn ode23_rejected_steps_shrink_by_at_most_half() {
        for direction in [-1., 1.] {
            let mut p = problem(OdeMethod::BogackiShampine23, vec![0., direction], vec![1.]);
            p.htry = Some(0.7);
            p.hmax = 1.;
            let mut times = Vec::new();
            let out = ode_integrate(&p, vec![-10. * direction], &mut |t, y| {
                times.push(t);
                Ok(vec![-10. * direction * y[0]])
            })
            .unwrap();
            assert!(out.failed >= 2);
            assert!(out.failure.is_none());
            // Each BS attempt makes three calls; its last call is at its end.
            // These first three attempts all start at zero. The first failure
            // must halve despite its large error, and the next failure halves.
            assert_eq!(times[2], direction * 0.7);
            assert_eq!(times[5], direction * 0.35);
            assert_eq!(times[8], direction * 0.175);
            assert_eq!(out.evals, 1 + times.len());
            assert_eq!(*out.t.last().unwrap(), direction);
        }
    }

    #[test]
    fn rejected_final_step_is_not_stretched_again() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            for direction in [-1., 1.] {
                let t0 = 1e6;
                let spacing = ode_eps(t0);
                let tfinal = t0 + direction * 17. * spacing;
                let mut p = problem(method, vec![t0, tfinal], vec![0.]);
                p.hmax = 18. * spacing;
                p.htry = Some(p.hmax);
                p.refine = 1;
                p.rtol = 0.002;
                p.threshold.fill(100.);
                // A jump at tfinal rejects the first 17-ulp attempt. Its retry
                // is floored at hmin=16 ulps, within 10% of the remaining span.
                // It must accept that shorter interval before trying tfinal.
                let out = ode_integrate(&p, vec![0.], &mut |t, _| {
                    Ok(vec![if t == tfinal { 1. / spacing } else { 0. }])
                })
                .unwrap();
                assert!(out.failure.is_none(), "{:?}", out.failure);
                assert_eq!(out.failed, 1);
                assert_eq!(out.steps, 2);
                assert_eq!(out.t, [t0, t0 + direction * 16. * spacing, tfinal]);
                assert_eq!(out.evals, 1 + 3 * method.evaluations());

                // With no rejection, the fresh 16-ulp step may still stretch.
                p.htry = Some(16. * spacing);
                let out = ode_integrate(&p, vec![0.], &mut |_, _| Ok(vec![0.])).unwrap();
                assert_eq!(out.failed, 0);
                assert_eq!(out.steps, 1);
                assert_eq!(out.t, [t0, tfinal]);

                // If the failed clipped interval is already below hmin,
                // there is no smaller admissible retry. Never overshoot it.
                p.tfinal = t0 + direction * 8. * spacing;
                p.tspan[1] = p.tfinal;
                p.htry = Some(p.hmax);
                let out = ode_integrate(&p, vec![0.], &mut |t, _| {
                    assert!(direction * (t - p.tfinal) <= 0.);
                    Ok(vec![if t == p.tfinal { 10. / spacing } else { 0. }])
                })
                .unwrap();
                assert_eq!(out.failed, 1);
                assert_eq!(out.steps, 0);
                assert_eq!(out.failure, Some((t0, 16. * spacing)));
                assert_eq!(out.t, [t0]);
            }
        }
    }

    #[test]
    fn minimum_step_failure_returns_last_accepted_point() {
        let mut p = problem(OdeMethod::DormandPrince45, vec![0., 2.], vec![1.]);
        p.rtol = 1e-3;
        p.threshold.fill(1e-3);
        let out = ode_integrate(&p, vec![1.], &mut |_, y| Ok(vec![y[0] * y[0]])).unwrap();
        assert!(out.failed > 0);
        let (t, hmin) = out.failure.unwrap();
        assert!((0.99..1.01).contains(&t));
        assert_eq!(hmin, 16. * ode_eps(t));
        assert_eq!(*out.t.last().unwrap(), t);
    }

    #[test]
    fn infinity_and_error_norms_propagate_nan() {
        assert_eq!(ode_inf_norm([].into_iter()), 0.);
        assert_eq!(ode_inf_norm([-3., 2.].into_iter()), 3.);
        assert_eq!(
            ode_inf_norm([1., f64::NEG_INFINITY].into_iter()),
            f64::INFINITY
        );
        for values in [[f64::NAN, 1.], [1., f64::NAN], [f64::NAN, f64::INFINITY]] {
            assert!(ode_inf_norm(values.into_iter()).is_nan());
            assert!(ode_error_norm(&values, &[1.; 2], &[2.; 2], &[1e-3; 2]).is_nan());
        }
    }

    #[test]
    fn mixed_nan_derivatives_do_not_shrink_or_reject_steps() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            for (tfinal, squared) in [(2., true), (1., false)] {
                let mut p = problem(method, vec![0., tfinal], vec![1.; 2]);
                p.rtol = 1e-3;
                p.threshold.fill(1e-3);
                let mut f = |_: f64, y: &[f64]| {
                    Ok(vec![
                        if squared { y[0] * y[0] } else { -30. * y[0] },
                        f64::NAN,
                    ])
                };
                let f0 = f(p.t0, &p.y0).unwrap();
                let out = ode_integrate(&p, f0, &mut f).unwrap();
                // A NaN initial slope leaves the initial trial at MaxStep;
                // NaN error estimates must also accept all subsequent steps.
                assert_eq!(out.steps, 10);
                assert_eq!(out.t.len(), 41);
                assert_eq!(out.failed, 0);
                assert_eq!(out.evals, 1 + 10 * method.evaluations());
                assert!(out.failure.is_none());
                assert_eq!(*out.t.last().unwrap(), tfinal);
                assert!(
                    out.y
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .skip(1)
                        .all(|y| y[1].is_nan())
                );
            }
        }
    }

    #[test]
    fn max_step_below_minimum_retry_step_keeps_requested_grid() {
        for (span, max_step, points) in [(2e-8, Some(1e-9), 81), (1e-8, None, 41)] {
            let mut p = problem(OdeMethod::DormandPrince45, vec![1e6, 1e6 + span], vec![1.]);
            p.rtol = 1e-3;
            p.threshold.fill(1e-3);
            if let Some(hmax) = max_step {
                p.hmax = hmax;
            }
            assert!(p.hmax < 16. * ode_eps(p.t0));
            let out = ode_integrate(&p, vec![-1.], &mut |_, y| Ok(vec![-y[0]])).unwrap();
            assert_eq!(out.t.len(), points);
            assert_eq!(out.steps, (points - 1) / p.refine);
            assert_eq!(out.failed, 0);
            assert!(out.failure.is_none());
            assert_eq!(*out.t.last().unwrap(), p.tfinal);
            assert!(
                out.t
                    .windows(2)
                    .all(|w| w[1] >= w[0] && w[1] - w[0] <= p.hmax)
            );
        }
    }

    #[test]
    fn permitted_nan_inputs_do_not_stall_step_control() {
        for method in [OdeMethod::DormandPrince45, OdeMethod::BogackiShampine23] {
            let mut p = problem(method, vec![0., 1.], vec![1.]);
            // Argument parsing permits NaN RelTol; comparisons must retain
            // that existing policy rather than forcing endless rejected steps.
            p.rtol = f64::NAN;
            p.threshold.fill(f64::NAN);
            let out = ode_integrate(&p, vec![-1.], &mut |_, y| Ok(vec![-y[0]])).unwrap();
            assert!(out.failure.is_none());
            assert_eq!(out.failed, 0);
            assert_eq!(*out.t.last().unwrap(), 1.);
            p.rtol = 1e-3;
            p.threshold.fill(1e-3);
            p.y0.fill(f64::NAN);
            let out = ode_integrate(&p, vec![f64::NAN], &mut |_, _| Ok(vec![f64::NAN])).unwrap();
            assert!(out.failure.is_none());
            assert!(out.y.iter().all(|v| v.is_nan()));
            assert_eq!(*out.t.last().unwrap(), 1.);
        }
    }
}
