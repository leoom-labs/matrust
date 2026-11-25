//! LTI responses, independent of the desktop. Continuous propagation uses block
//! exponentials (Van Loan, Computing Integrals Involving the Matrix Exponential,
//! IEEE TAC 23, 1978, equations 1.1 and Theorem 1). FOH adds an input derivative
//! block. Frequency evaluation uses orthogonal Hessenberg reduction followed by
//! pivoted Hessenberg solves (Laub, Efficient Multivariable Frequency Response
//! Computations, IEEE TAC 26, 1981, pp. 407-408). No explicit inverses.
use super::{
    Engine, Execution, Value,
    control_lti::{self, Lti, Model},
};
use crate::{Array, Complex64 as C, ComplexMatrix, Matrix, Result, StructArray};
use nalgebra::DMatrix;
use rayon::prelude::*;
use std::{
    f64::consts::PI,
    sync::{Arc, atomic::Ordering},
};

mod delayed_time;
mod freq;
use delayed_time::{CtrDelayedTime, ctr_delayed_time, ctr_time_paths};

const CTR_POINTS: usize = 10_000;
const CTR_ENTRIES: usize = 4_000_000;
const CTR_WORK: usize = 500_000_000;
const CTR_ZERO_RATE: f64 = 1e-10;
const CTR_IMAGINARY_REL_RATE: f64 = 1e-10;
const CTR_NAMES: &[&str] = &[
    "step",
    "impulse",
    "initial",
    "lsim",
    "stepinfo",
    "bode",
    "nyquist",
    "freqresp",
    "evalfr",
    "margin",
    "pzmap",
    "zplane",
    "nichols",
    "sigma",
    "allmargin",
    "ngrid",
];
pub(super) fn handles(name: &str) -> bool {
    CTR_NAMES.contains(&name)
}
pub(super) fn plots(name: &str) -> bool {
    matches!(
        name,
        "step"
            | "impulse"
            | "initial"
            | "lsim"
            | "bode"
            | "nyquist"
            | "margin"
            | "pzmap"
            | "zplane"
            | "nichols"
            | "sigma"
            | "ngrid"
    )
}
fn ctr_real(v: &Value) -> Result<Matrix> {
    match v {
        Value::Matrix(m) if m.len() <= CTR_ENTRIES && m.data().iter().all(|x| x.is_finite()) => {
            Ok(m.clone())
        }
        _ => Err(
            "Control response inputs must be finite real double arrays within the size bound"
                .into(),
        ),
    }
}
fn ctr_vector(v: &Value) -> Result<Vec<f64>> {
    let m = ctr_real(v)?;
    if m.is_empty() || (m.rows() != 1 && m.cols() != 1) || m.len() > CTR_POINTS {
        return Err("Control response vector must have 1..10000 entries".into());
    }
    Ok(m.data().to_vec())
}
fn ctr_lti(e: &Engine, v: &Value) -> Result<Lti> {
    if let Value::Container(a) = v
        && let Some(s) = control_lti::decode(e, a)?
    {
        return Ok(s);
    }
    Err("Expected an LTI model created by tf, zpk or ss".into())
}
fn ctr_matrix(m: &DMatrix<f64>) -> Result<Matrix> {
    Matrix::from_column_slice(m.nrows(), m.ncols(), m.as_slice())
}
fn ctr_dm(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}
fn ctr_column(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(v.len(), 1, v)?))
}
fn ctr_poles(e: &mut Engine, s: &Value, out: &mut Execution) -> Result<Vec<C>> {
    match e.call("pole", vec![s.clone()], out)? {
        Value::Complex(m) => Ok(m.data().to_vec()),
        Value::Matrix(m) => Ok(m.data().iter().map(|&x| C::new(x, 0.)).collect()),
        _ => Err("Invalid LTI pole result".into()),
    }
}
// Admit work before allocating trajectories or augmented matrices. Bounds also
// keep each non-cancellable library factorization reasonably short.
fn ctr_admit(n: usize, inputs: usize, outputs: usize, traces: usize, samples: usize) -> Result<()> {
    if n > 256
        || inputs > 64
        || outputs > 64
        || samples.saturating_mul(traces).saturating_mul(n + outputs) > CTR_ENTRIES
        || samples
            .saturating_mul(n)
            .saturating_mul(n + inputs + outputs)
            .saturating_mul(traces)
            > CTR_WORK
    {
        return Err(
            "Control response exceeds 256 states, 64 ports, 4 million entries or work bound".into(),
        );
    }
    Ok(())
}
fn ctr_time(
    e: &mut Engine,
    s: &Value,
    ts: f64,
    delay: f64,
    arg: Option<&Value>,
    out: &mut Execution,
) -> Result<Vec<f64>> {
    if let Some(arg) = arg {
        let v = ctr_vector(arg)?;
        if v.len() > 1 {
            ctr_check_time(&v, ts)?;
            return Ok(v);
        }
    }
    let poles = ctr_poles(e, s, out)?;
    let rates: Vec<C> = poles
        .iter()
        .map(|p| if ts > 0. { p.ln() / ts } else { *p })
        .collect();
    // Eig/log rounding can give undamped poles tiny negative real parts.
    // Classify damping relative to the rate magnitude after the discrete log,
    // retaining the absolute integrator threshold from the automatic grid rule.
    let stable = rates
        .iter()
        .filter(|p| p.re < -CTR_IMAGINARY_REL_RATE * p.norm() && p.norm() > CTR_ZERO_RATE)
        .map(|p| 7. / p.re.abs())
        .fold(0., f64::max);
    let fallback = rates
        .iter()
        .filter(|p| {
            p.re >= -CTR_IMAGINARY_REL_RATE * p.norm()
                && p.norm() > CTR_ZERO_RATE
                && p.norm().is_finite()
        })
        .map(|p| 10. / p.norm())
        .fold(0., f64::max);
    let final_time = match arg {
        Some(v) => ctr_real(v)?.as_scalar()?,
        None => stable
            .max(fallback)
            .max(if rates.iter().any(|p| p.norm() <= CTR_ZERO_RATE) {
                10.
            } else {
                0.
            }),
    };
    let final_time = if arg.is_none() && final_time == 0. {
        if ts > 0. { 100. * ts } else { 10. }
    } else {
        final_time
    };
    let final_time = final_time + if arg.is_none() { delay } else { 0. };
    if !final_time.is_finite() || final_time <= 0. {
        return Err("Final time must be positive and finite".into());
    }
    let fastest = rates
        .iter()
        .map(|p| p.norm())
        .filter(|v| v.is_finite())
        .fold(0., f64::max);
    let (count, dt) = if ts > 0. {
        let requested = (final_time / ts + 1e-10).floor();
        // A default discrete grid keeps Ts and bounds its horizon, rather than
        // changing Ts to hit an arbitrary pole-derived final time.
        let intervals = if arg.is_none() {
            requested.clamp(99., 9999.)
        } else {
            requested
        };
        if !(1. ..CTR_POINTS as f64).contains(&intervals) {
            return Err("Discrete time grid requires 2..10000 samples at Ts".into());
        }
        (intervals as usize + 1, ts)
    } else {
        let count = ((final_time * fastest * 25.).ceil() as usize)
            .saturating_add(1)
            .clamp(100, CTR_POINTS);
        (count, final_time / (count - 1) as f64)
    };
    let t: Vec<f64> = (0..count).map(|k| k as f64 * dt).collect();
    ctr_check_time(&t, ts)?;
    Ok(t)
}
fn ctr_check_time(t: &[f64], ts: f64) -> Result<()> {
    if t.len() < 2 || t.len() > CTR_POINTS || t[0] != 0. {
        return Err("Time must be a uniform vector starting at zero with 2..10000 samples".into());
    }
    let dt = t[1] - t[0];
    if dt <= 0.
        || !dt.is_finite()
        || t.iter()
            .enumerate()
            .any(|(k, v)| !v.is_finite() || (*v - k as f64 * dt).abs() > 1e-9 * dt.max(v.abs()))
    {
        return Err("Time must be finite, increasing and uniformly spaced".into());
    }
    if ts > 0. && (dt - ts).abs() > 1e-9 * ts {
        return Err("Discrete response time spacing must equal Ts".into());
    }
    Ok(())
}
struct CtrPropagation {
    a: DMatrix<f64>,
    b0: DMatrix<f64>,
    b1: DMatrix<f64>,
}
fn ctr_propagation(
    e: &mut Engine,
    a: &Matrix,
    b: &Matrix,
    ts: f64,
    dt: f64,
    foh: bool,
    out: &mut Execution,
) -> Result<CtrPropagation> {
    let (n, m) = (a.rows(), b.cols());
    if ts > 0. {
        return Ok(CtrPropagation {
            a: ctr_dm(a),
            b0: ctr_dm(b),
            b1: DMatrix::zeros(n, m),
        });
    }
    let size = n + if foh { 2 * m } else { m };
    let mut block = DMatrix::zeros(size, size);
    block.view_mut((0, 0), (n, n)).copy_from(&(ctr_dm(a) * dt));
    block.view_mut((0, n), (n, m)).copy_from(&(ctr_dm(b) * dt));
    if foh {
        for j in 0..m {
            block[(n + j, n + m + j)] = 1.;
        }
    }
    let Value::Matrix(exp) = e.call("expm", vec![Value::Matrix(ctr_matrix(&block)?)], out)? else {
        return Err("Expected real matrix exponential".into());
    };
    if exp.data().iter().any(|x| !x.is_finite()) {
        return Err("Response discretization overflowed".into());
    }
    let exp = ctr_dm(&exp);
    let ad = exp.view((0, 0), (n, n)).into_owned();
    let bd = exp.view((0, n), (n, m)).into_owned();
    let b1 = if foh {
        exp.view((0, n + m), (n, m)).into_owned()
    } else {
        DMatrix::zeros(n, m)
    };
    Ok(CtrPropagation {
        a: ad,
        b0: bd - &b1,
        b1,
    })
}
struct CtrTime {
    y: Matrix,
    t: Vec<f64>,
    x: Matrix,
    outputs: usize,
    inputs: usize,
}
fn ctr_time_response(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    save_states: bool,
    out: &mut Execution,
) -> Result<CtrTime> {
    let required = match name {
        "lsim" => 3,
        "initial" => 2,
        _ => 1,
    };
    if args.len() < required || args.len() > if name == "lsim" { 5 } else { required + 1 } {
        return Err(format!("Invalid arguments for {name}"));
    }
    let s = ctr_lti(e, &args[0])?;
    let time_arg = if name == "lsim" {
        args.get(2)
    } else {
        args.get(required)
    };
    let ts = if s.ts == -1. {
        match time_arg {
            Some(Value::Matrix(m)) if m.len() > 1 => {
                let dt = m.data()[1] - m.data()[0];
                if !dt.is_finite() || dt <= 0. {
                    return Err("Time must be finite, increasing and uniformly spaced".into());
                }
                dt
            }
            _ => s.sample_period(),
        }
    } else {
        s.ts
    };
    let delays = s.delays().map(|d| d * if ts > 0. { ts } else { 1. });
    let [_, output_delay, _] = delays;
    let max_path_delay = s.internal_paths().iter().map(Lti::delay).fold(0., f64::max)
        * if ts > 0. { ts } else { 1. };
    if name == "initial" && !matches!(s.model, Model::Ss(..)) {
        return Err("initial requires a state-space model".into());
    }
    let (a, b, c, d) = e.ctl_ss(&s, out)?;
    let (n, m, p) = (a.rows(), b.cols(), c.rows());
    if p == 0 || (m == 0 && name != "initial") {
        return Err(format!("{name} requires at least one input and one output"));
    }
    let traces = if matches!(name, "step" | "impulse") {
        m
    } else {
        1
    };
    ctr_admit(n, m, p, traces, 1)?;
    let mut foh = false;
    let mut x0 = None;
    if name == "lsim" {
        let mut interpolation_seen = false;
        for v in &args[3..] {
            if v.is_character() || matches!(v, Value::QuotedString(_)) {
                let method = e.text_value(v.clone())?;
                if interpolation_seen || !matches!(method.as_str(), "zoh" | "foh") {
                    return Err("lsim accepts one interpolation option, 'zoh' or 'foh'".into());
                }
                interpolation_seen = true;
                foh = method == "foh";
            } else if x0.is_none() {
                x0 = Some(v);
            } else {
                return Err("lsim accepts at most one initial state".into());
            }
        }
    } else if name == "initial" {
        x0 = Some(&args[1]);
    }
    let t = if name == "lsim" {
        let t = ctr_vector(&args[2])?;
        ctr_check_time(&t, ts)?;
        t
    } else {
        ctr_time(
            e,
            &args[0],
            ts,
            if name == "initial" {
                output_delay + max_path_delay
            } else {
                delays.iter().sum::<f64>() + max_path_delay
            },
            args.get(required),
            out,
        )?
    };
    ctr_admit(n, m, p, traces, t.len())?;
    let initial = if let Some(v) = x0 {
        let v = ctr_real(v)?;
        if v.len() != n || (n > 0 && v.rows() != 1 && v.cols() != 1) {
            return Err("Initial state must be a vector with one entry per state".into());
        }
        DMatrix::from_column_slice(n, 1, v.data())
    } else {
        DMatrix::zeros(n, traces)
    };
    let u = if name == "lsim" {
        let u = ctr_real(&args[1])?;
        if m == 1 && u.len() == t.len() && (u.rows() == 1 || u.cols() == 1) {
            u.reshape(t.len(), 1)?
        } else if u.rows() == t.len() && u.cols() == m {
            u
        } else {
            return Err(
                "lsim input must have one row per time sample and one column per input".into(),
            );
        }
    } else {
        Matrix::filled(0, 0, 0.)?
    };
    if s.has_internal_delays() {
        return ctr_time_paths(e, &s, name, t, &u, initial, foh, save_states, out);
    }
    let propagation = ctr_propagation(e, &a, &b, ts, t[1] - t[0], foh, out)?;
    if delays != [0.; 3] {
        return ctr_delayed_time(
            e,
            CtrDelayedTime {
                a: &a,
                b: &b,
                c: &c,
                d: &d,
                ts,
                t,
                foh,
                propagation,
                delays,
                save_states,
            },
            name,
            &u,
            initial,
            out,
        );
    }
    let mut state = if name == "impulse" && ts == 0. {
        ctr_dm(&b)
    } else {
        initial
    };
    let (c, d) = (ctr_dm(&c), ctr_dm(&d));
    let input = |k: usize| -> DMatrix<f64> {
        match name {
            "step" => DMatrix::identity(m, m),
            "impulse" if ts > 0. && k == 0 => DMatrix::identity(m, m) / ts,
            "lsim" => DMatrix::from_fn(m, 1, |i, _| u.get(k, i)),
            _ => DMatrix::zeros(m, traces),
        }
    };
    let mut y = vec![0.; t.len() * p * traces];
    let mut x = vec![0.; if save_states { t.len() * n * traces } else { 0 }];
    for k in 0..t.len() {
        e.check_cancel()?;
        let uk = input(k);
        let output = &c * &state + &d * &uk;
        if output.iter().chain(state.iter()).any(|v| !v.is_finite()) {
            return Err("Time response overflowed".into());
        }
        for j in 0..traces {
            for i in 0..p {
                y[k + t.len() * (i + p * j)] = output[(i, j)];
            }
            if save_states {
                for i in 0..n {
                    x[k + t.len() * (i + n * j)] = state[(i, j)];
                }
            }
        }
        if k + 1 < t.len() {
            state = &propagation.a * state + &propagation.b0 * uk + &propagation.b1 * input(k + 1);
        }
    }
    Ok(CtrTime {
        y: Matrix::from_column_vec(t.len(), p * traces, y)?,
        x: Matrix::from_column_vec(t.len(), if save_states { n * traces } else { 0 }, x)?,
        t,
        outputs: p,
        inputs: traces,
    })
}

// Frequency matrices use nalgebra's proven Householder Hessenberg reduction.
// A shifted upper Hessenberg matrix needs only one elimination per column;
// partial pivoting exchanges adjacent rows. Cost O(n^2(nu+1)), not O(n^3).
enum CtrFrequency {
    Paths(Vec<CtrEvaluator>),
    Tf(Vec<(Matrix, Matrix)>),
    Ss {
        h: DMatrix<C>,
        b: DMatrix<C>,
        c: DMatrix<C>,
        d: DMatrix<C>,
    },
}
struct CtrEvaluator {
    model: CtrFrequency,
    ts: f64,
    delay: f64,
    inputs: usize,
    outputs: usize,
}
fn ctr_evaluator(e: &mut Engine, s: &Lti, out: &mut Execution) -> Result<CtrEvaluator> {
    if s.has_internal_delays() {
        ctr_admit(s.states(), 1, 1, 1, 1)?;
        let paths = s
            .internal_paths()
            .iter()
            .map(|path| ctr_evaluator(e, path, out))
            .collect::<Result<Vec<_>>>()?;
        return Ok(CtrEvaluator {
            model: CtrFrequency::Paths(paths),
            ts: s.sample_period(),
            delay: s.delay(),
            inputs: 1,
            outputs: 1,
        });
    }
    let (inputs, outputs) = s.ports();
    if inputs == 0 || outputs == 0 {
        return Err("Frequency response requires at least one input and one output".into());
    }
    ctr_admit(s.states(), inputs, outputs, 1, 1)?;
    let model = match &s.model {
        Model::Ss(a, b, c, d) => {
            let (q, h) = if a.rows() > 1 {
                ctr_dm(a).hessenberg().unpack()
            } else {
                (DMatrix::identity(a.rows(), a.rows()), ctr_dm(a))
            };
            CtrFrequency::Ss {
                h: h.map(|x| C::new(x, 0.)),
                b: (q.transpose() * ctr_dm(b)).map(|x| C::new(x, 0.)),
                c: (ctr_dm(c) * q).map(|x| C::new(x, 0.)),
                d: ctr_dm(d).map(|x| C::new(x, 0.)),
            }
        }
        _ => CtrFrequency::Tf(control_lti::frequency_coefficients(e, s, out)?),
    };
    Ok(CtrEvaluator {
        model,
        ts: s.sample_period(),
        delay: s.delay(),
        inputs,
        outputs,
    })
}

fn ctr_hessenberg_solve(h: &DMatrix<C>, b: &DMatrix<C>, z: C) -> Result<DMatrix<C>> {
    let n = h.nrows();
    let mut a = -h;
    let mut x = b.clone();
    for i in 0..n {
        a[(i, i)] += z;
    }
    for k in 0..n.saturating_sub(1) {
        if a[(k + 1, k)].norm() > a[(k, k)].norm() {
            a.swap_rows(k, k + 1);
            x.swap_rows(k, k + 1);
        }
        if a[(k, k)].norm() == 0. {
            return Err("Frequency coincides with a system pole".into());
        }
        let factor = a[(k + 1, k)] / a[(k, k)];
        a[(k + 1, k)] = C::new(0., 0.);
        for j in k + 1..n {
            let top = a[(k, j)];
            a[(k + 1, j)] -= factor * top;
        }
        for j in 0..x.ncols() {
            let top = x[(k, j)];
            x[(k + 1, j)] -= factor * top;
        }
    }
    for i in (0..n).rev() {
        if a[(i, i)].norm() == 0. {
            return Err("Frequency coincides with a system pole".into());
        }
        for j in 0..x.ncols() {
            let mut v = x[(i, j)];
            for k in i + 1..n {
                v -= a[(i, k)] * x[(k, j)];
            }
            x[(i, j)] = v / a[(i, i)];
        }
    }
    Ok(x)
}
impl CtrEvaluator {
    fn ctr_at_rational(&self, z: C) -> Result<Vec<C>> {
        let result = match &self.model {
            CtrFrequency::Paths(paths) => ctr_sum_paths(paths, |path| path.ctr_at(z))?,
            CtrFrequency::Tf(channels) => {
                let horner = |p: &Matrix| p.data().iter().fold(C::new(0., 0.), |v, &a| v * z + a);
                channels
                    .iter()
                    .map(|(n, d)| {
                        let den = horner(d);
                        if den == C::new(0., 0.) {
                            return Err("Frequency coincides with a system pole".into());
                        }
                        Ok(horner(n) / den)
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            CtrFrequency::Ss { h, b, c, d } => {
                (c * ctr_hessenberg_solve(h, b, z)? + d).as_slice().to_vec()
            }
        };
        if result
            .iter()
            .any(|v| !v.re.is_finite() || !v.im.is_finite())
        {
            return Err("Frequency response overflowed".into());
        }
        Ok(result)
    }
    fn ctr_apply_delay(mut h: Vec<C>, factor: C) -> Result<Vec<C>> {
        for value in &mut h {
            *value *= factor;
            if !value.re.is_finite() || !value.im.is_finite() {
                return Err("Delayed frequency response overflowed".into());
            }
        }
        Ok(h)
    }
    fn ctr_at(&self, z: C) -> Result<Vec<C>> {
        let h = self.ctr_at_rational(z)?;
        if self.delay == 0. {
            return Ok(h);
        }
        let factor = if self.ts > 0. {
            if z == C::new(0., 0.) {
                return Err("Frequency coincides with a discrete delay pole at zero".into());
            }
            z.powf(-self.delay)
        } else {
            (-z * self.delay).exp()
        };
        Self::ctr_apply_delay(h, factor)
    }
    fn ctr_delay_seconds(&self) -> f64 {
        self.delay * if self.ts == 0. { 1. } else { self.ts }
    }
    fn ctr_longest_delay(&self) -> f64 {
        let internal = match &self.model {
            CtrFrequency::Paths(paths) => {
                paths.iter().map(Self::ctr_delay_seconds).fold(0., f64::max)
            }
            _ => 0.,
        };
        self.ctr_delay_seconds() + internal
    }
    fn ctr_states(&self) -> usize {
        match &self.model {
            CtrFrequency::Paths(paths) => paths.iter().map(Self::ctr_states).sum(),
            CtrFrequency::Ss { h, .. } => h.nrows(),
            _ => 0,
        }
    }
    fn ctr_coefficient_count(&self) -> usize {
        match &self.model {
            CtrFrequency::Tf(channels) => channels.iter().map(|(n, d)| n.len() + d.len()).sum(),
            CtrFrequency::Paths(paths) => paths.iter().map(Self::ctr_coefficient_count).sum(),
            CtrFrequency::Ss { .. } => 0,
        }
    }
    fn ctr_frequency_base(&self, w: f64) -> Result<Vec<C>> {
        if let CtrFrequency::Paths(paths) = &self.model {
            return ctr_sum_paths(paths, |path| path.ctr_frequency(w));
        }
        let z = if self.ts == 0. {
            C::new(0., w)
        } else if w == PI / self.ts {
            C::new(-1., 0.)
        } else {
            C::from_polar(1., w * self.ts)
        };
        self.ctr_at_rational(z)
    }
    fn ctr_factor(&self, w: f64) -> C {
        if self.ts > 0. && w == PI / self.ts {
            C::new(if self.delay % 2. == 0. { 1. } else { -1. }, 0.)
        } else {
            C::from_polar(1., -w * self.ctr_delay_seconds())
        }
    }
    fn ctr_frequency(&self, w: f64) -> Result<Vec<C>> {
        let h = self.ctr_frequency_base(w)?;
        if self.delay == 0. {
            return Ok(h);
        }
        Self::ctr_apply_delay(h, self.ctr_factor(w))
    }
}
fn ctr_sum_paths(
    paths: &[CtrEvaluator],
    evaluate: impl Fn(&CtrEvaluator) -> Result<Vec<C>>,
) -> Result<Vec<C>> {
    let mut value = C::new(0., 0.);
    for path in paths {
        value += evaluate(path)?[0];
    }
    if !value.re.is_finite() || !value.im.is_finite() {
        return Err("Frequency response overflowed".into());
    }
    Ok(vec![value])
}
fn ctr_frequency_grid(
    e: &mut Engine,
    value: &Value,
    s: &Lti,
    arg: Option<&Value>,
    out: &mut Execution,
) -> Result<Vec<f64>> {
    let ts = s.sample_period();
    if let Some(v) = arg {
        let w = ctr_vector(v)?;
        if w.iter().any(|&x| x < 0.) || w.windows(2).any(|x| x[1] <= x[0]) {
            return Err("Frequencies must be nonnegative and strictly increasing".into());
        }
        if ts > 0. && w[w.len() - 1] > PI / ts * (1. + 1e-12) {
            return Err(
                "Discrete frequency grid must not exceed the Nyquist frequency pi/Ts".into(),
            );
        }
        return Ok(w);
    }
    let mut roots = ctr_poles(e, value, out)?;
    // MIMO transmission zeros are not available from ctl_zero yet: the automatic
    // MIMO grid uses poles alone, explicitly documented in the help pages.
    if !s.has_internal_delays() && s.ports() == (1, 1) {
        match e.call("zero", vec![value.clone()], out)? {
            Value::Complex(z) => roots.extend_from_slice(z.data()),
            Value::Matrix(z) => roots.extend(z.data().iter().map(|&x| C::new(x, 0.))),
            _ => return Err("Invalid LTI zero result".into()),
        }
    }
    let scales: Vec<f64> = roots
        .iter()
        .map(|p| {
            if ts > 0. {
                p.ln().norm() / ts
            } else {
                p.norm()
            }
        })
        .filter(|x| x.is_finite() && *x > CTR_ZERO_RATE)
        .collect();
    let lo = if scales.is_empty() {
        -2.
    } else {
        scales
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
            .log10()
            .floor()
            - 1.
    };
    let hi = if scales.is_empty() {
        2.
    } else {
        scales.iter().copied().fold(0., f64::max).log10().ceil() + 1.
    };
    let max = if ts > 0. { (PI / ts).log10() } else { hi };
    let min = lo.min(max - 2.);
    let count = 1000;
    let mut w: Vec<f64> = (0..count)
        .map(|k| 10f64.powf(min + (max - min) * k as f64 / (count - 1) as f64))
        .collect();
    if ts > 0. {
        w[count - 1] = PI / ts;
    }
    if w.iter().any(|x| !x.is_finite() || *x == 0.) {
        return Err("Automatic frequency grid overflowed; supply an explicit grid".into());
    }
    Ok(w)
}

// Coefficients in x=w^2 of Re(a(jw)*b(-jw)), or Im(...)/w.
// Inputs are real coefficients in ascending powers, with a common scale.
fn ctr_crossing_polynomial(e: &Engine, a: &[f64], b: &[f64], odd: usize) -> Result<Vec<f64>> {
    let mut p = vec![0.; (a.len() + b.len() - odd) / 2];
    for (i, &ai) in a.iter().enumerate() {
        e.check_cancel()?;
        for (j, &bj) in b.iter().enumerate() {
            if (i + j) % 2 != odd {
                continue;
            }
            let product = ai * bj;
            if product == 0. && ai != 0. && bj != 0. {
                return Err(
                    "Margin crossing coefficients underflowed; use sampled frequency data".into(),
                );
            }
            // i^(i-j) has a real/imaginary component of exactly +1 or -1.
            let sign = if (i + 4 - j % 4) % 4 < 2 { 1. } else { -1. };
            p[(i + j) / 2] += sign * product;
        }
    }
    Ok(p)
}

fn ctr_positive_root_log_bounds(p: &[f64]) -> Option<(f64, f64)> {
    // Descartes' rule: no positive root when all nonzero coefficients agree.
    // Remove powers of x; zero frequency is outside the logarithmic search.
    if !p.iter().any(|&x| x > 0.) || !p.iter().any(|&x| x < 0.) {
        return None;
    }
    let first = p.iter().position(|&x| x != 0.)?;
    let last = p.iter().rposition(|&x| x != 0.)?;
    let p = &p[first..=last];
    let n = p.len() - 1;
    // For R=2*max_k |a_k/a_n|^(1/(n-k)), every root has |x|<=R:
    // outside R the lower terms / leading term sum to less than sum 2^-k=1.
    // Apply the same bound to x^n*p(1/x) for the lower bound. Logarithms
    // avoid overflow in coefficient ratios. See DLMF 1.11(ii) for Descartes
    // and the reciprocal-polynomial identity; the bound follows as above.
    let upper = p[..n]
        .iter()
        .enumerate()
        .map(|(k, a)| (a.abs().log10() - p[n].abs().log10()) / (n - k) as f64)
        .fold(f64::NEG_INFINITY, f64::max)
        + 2f64.log10();
    let inverse_upper = p[1..]
        .iter()
        .enumerate()
        .map(|(k, a)| (a.abs().log10() - p[0].abs().log10()) / (k + 1) as f64)
        .fold(f64::NEG_INFINITY, f64::max)
        + 2f64.log10();
    Some((-inverse_upper, upper))
}

fn ctr_margin_grid(e: &Engine, (num, den): &(Matrix, Matrix), w: Vec<f64>) -> Result<Vec<f64>> {
    let scale = num
        .data()
        .iter()
        .chain(den.data())
        .map(|x| x.abs())
        .fold(0., f64::max);
    let normalized = |p: &Matrix| -> Result<Vec<f64>> {
        p.data()
            .iter()
            .rev()
            .map(|&v| {
                let scaled = v / scale;
                if !scaled.is_finite() || (scaled == 0. && v != 0.) {
                    Err("Margin coefficients cannot be scaled; use sampled frequency data".into())
                } else {
                    Ok(scaled)
                }
            })
            .collect()
    };
    let (num, den) = (normalized(num)?, normalized(den)?);
    let mut gain = ctr_crossing_polynomial(e, &num, &num, 0)?;
    let denominator = ctr_crossing_polynomial(e, &den, &den, 0)?;
    gain.resize(gain.len().max(denominator.len()), 0.);
    for (coefficient, d) in gain.iter_mut().zip(denominator) {
        *coefficient -= d;
    }
    let phase = ctr_crossing_polynomial(e, &num, &den, 1)?;
    let (mut lo, mut hi) = (w[0].log10(), w[w.len() - 1].log10());
    let spacing = (hi - lo) / (w.len() - 1) as f64;
    // Every finite nonzero unity-gain or real-axis crossing lies within the
    // coefficient bounds. Include a decade of padding, then retain at least
    // the original log-grid density; refine crossings against the evaluator.
    for coefficients in [&gain, &phase] {
        if let Some((lower, upper)) = ctr_positive_root_log_bounds(coefficients) {
            lo = lo.min((lower / 2.).floor() - 1.);
            hi = hi.max((upper / 2.).ceil() + 1.);
        }
    }
    if lo == w[0].log10() && hi == w[w.len() - 1].log10() {
        return Ok(w);
    }
    let intervals = ((hi - lo) / spacing).ceil();
    if !intervals.is_finite() || intervals >= CTR_POINTS as f64 {
        return Err(
            "Automatic margin search exceeds 10000 points; use sampled frequency data".into(),
        );
    }
    let count = intervals as usize + 1;
    let grid: Vec<_> = (0..count)
        .map(|k| 10f64.powf(lo + (hi - lo) * k as f64 / intervals))
        .collect();
    if grid.iter().any(|w| !w.is_finite() || *w == 0.) {
        return Err("Automatic margin search overflowed; use sampled frequency data".into());
    }
    Ok(grid)
}
fn ctr_delay_margin_grid(
    mut w: Vec<f64>,
    delay: f64,
    ts: f64,
    automatic: bool,
    resolve_paths: bool,
) -> Result<Vec<f64>> {
    if delay == 0. || w.len() < 2 {
        return Ok(w);
    }
    let period = (2. * PI) / delay;
    if automatic {
        let low = period / 64.;
        if low > 0. && low < w[0] {
            w.insert(0, low);
        }
        // Include several delay-induced crossings after the rational grid's
        // last characteristic frequency, even for a small delay.
        let end = 8. * period;
        if ts == 0. && end.is_finite() && end > w[w.len() - 1] {
            w.push(end);
        }
    }
    // An external delay has a known, unwrapped phase. Only a sum of paths
    // with different delays needs extra samples to resolve interference.
    if !resolve_paths {
        return Ok(w);
    }
    ctr_resolve_delay_grid(
        &w,
        period,
        "Delayed margin search exceeds 10000 points; use a narrower sampled frequency range",
    )
}

fn ctr_resolve_delay_grid(w: &[f64], period: f64, limit_error: &str) -> Result<Vec<f64>> {
    let mut dense = Vec::with_capacity(w.len());
    dense.push(w[0]);
    for interval in w.windows(2) {
        let [lo, hi] = [interval[0], interval[1]];
        let count = ((hi - lo) / (period / 8.)).ceil().max(1.);
        if count > (CTR_POINTS - dense.len()) as f64 {
            return Err(limit_error.into());
        }
        let count = count as usize;
        for j in 1..count {
            dense.push(lo + (hi - lo) * (j as f64 / count as f64));
        }
        dense.push(hi);
    }
    Ok(dense)
}
struct CtrSpectrum {
    h: Vec<C>,
    w: Vec<f64>,
    rational: Option<Vec<C>>,
    path_phase: Option<Vec<f64>>,
    delay: f64,
    outputs: usize,
    inputs: usize,
}
fn ctr_spectrum(
    e: &mut Engine,
    value: &Value,
    frequencies: Option<&Value>,
    unwrap_paths: bool,
    out: &mut Execution,
) -> Result<CtrSpectrum> {
    let s = ctr_lti(e, value)?;
    let eval = ctr_evaluator(e, &s, out)?;
    let w = freq::fq_grid(e, value, &s, frequencies, out)?;
    if unwrap_paths && let CtrFrequency::Paths(paths) = &eval.model {
        let delay = paths
            .iter()
            .map(CtrEvaluator::ctr_longest_delay)
            .fold(0., f64::max);
        if delay > 0. && w.len() > 1 {
            return ctr_sample_path_phase(e, &eval, w, delay);
        }
    }
    ctr_sample_spectrum(e, &eval, w)
}

fn ctr_sample_path_phase(
    e: &Engine,
    eval: &CtrEvaluator,
    w: Vec<f64>,
    delay: f64,
) -> Result<CtrSpectrum> {
    // Unwrap the SISO path sum on intermediate samples while keeping the
    // response on the caller's grid. The outer delay is applied analytically.
    let dense = ctr_resolve_delay_grid(
        &w,
        2. * PI / delay,
        "Delayed response phase needs more than 10000 points; use a narrower frequency range",
    )?;
    if dense.len() == w.len() {
        return ctr_sample_spectrum(e, eval, w);
    }
    let mut spectrum = ctr_sample_spectrum(e, eval, dense)?;
    let raw = spectrum.rational.as_deref().unwrap_or(&spectrum.h);
    let mut phase = ctr_unwrap_phase(raw, &spectrum.w, spectrum.delay);
    let mut k = 0;
    for (j, frequency) in spectrum.w.iter().enumerate() {
        if w.get(k) == Some(frequency) {
            spectrum.h[k] = spectrum.h[j];
            if let Some(base) = &mut spectrum.rational {
                base[k] = base[j];
            }
            phase[k] = phase[j];
            k += 1;
        }
    }
    debug_assert_eq!(k, w.len());
    spectrum.h.truncate(k);
    if let Some(base) = &mut spectrum.rational {
        base.truncate(k);
    }
    phase.truncate(k);
    spectrum.w = w;
    spectrum.path_phase = Some(phase);
    Ok(spectrum)
}

fn ctr_sample_spectrum(e: &Engine, eval: &CtrEvaluator, w: Vec<f64>) -> Result<CtrSpectrum> {
    let pairs = eval.inputs * eval.outputs;
    let n = eval.ctr_states();
    ctr_admit(n, eval.inputs, eval.outputs, 1, w.len())?;
    let work = w
        .len()
        .saturating_mul(n)
        .saturating_mul(n + eval.outputs)
        .saturating_mul(eval.inputs + 1)
        .saturating_add(w.len().saturating_mul(eval.ctr_coefficient_count()));
    if work > CTR_WORK {
        return Err("Frequency response exceeds solve work bound".into());
    }
    if pairs.saturating_mul(w.len()) > CTR_ENTRIES {
        return Err("Frequency response exceeds entry bound".into());
    }
    let mut h = vec![C::new(0., 0.); w.len() * pairs];
    let workers = crate::parallel::current_worker_count().min(64);
    // Measured on the release benchmark in examples/benchmark_control_responses.
    // Independent frequency batches reuse the caller's pool. There is no
    // inner parallel kernel or new pool, and at most 64 concurrent batches.
    if work >= 1_000_000 && workers > 1 && n >= 16 {
        let batch = w.len().div_ceil(workers);
        let mut samples = vec![C::new(0., 0.); h.len()];
        let cancel = &e.cancel;
        samples
            .par_chunks_mut(batch * pairs)
            .enumerate()
            .try_for_each(|(chunk, values)| -> Result<()> {
                for (k, row) in values.chunks_mut(pairs).enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Calculation cancelled".into());
                    }
                    row.copy_from_slice(&eval.ctr_frequency_base(w[chunk * batch + k])?);
                }
                Ok(())
            })?;
        for (k, row) in samples.chunks(pairs).enumerate() {
            if k.is_multiple_of(128) {
                e.check_cancel()?;
            }
            for (j, &v) in row.iter().enumerate() {
                h[k + j * w.len()] = v;
            }
        }
    } else {
        for (k, &wk) in w.iter().enumerate() {
            e.check_cancel()?;
            for (j, v) in eval.ctr_frequency_base(wk)?.into_iter().enumerate() {
                h[k + j * w.len()] = v;
            }
        }
    }
    let rational = if eval.delay == 0. {
        None
    } else {
        let base = h.clone();
        for (k, &frequency) in w.iter().enumerate() {
            e.check_cancel()?;
            let phase = frequency * eval.ctr_delay_seconds();
            if !phase.is_finite() || phase > f64::MAX / 180. {
                return Err("Delayed frequency response phase overflowed".into());
            }
            let factor = eval.ctr_factor(frequency);
            for j in 0..pairs {
                h[k + j * w.len()] *= factor;
                let value = h[k + j * w.len()];
                if !value.re.is_finite() || !value.im.is_finite() {
                    return Err("Delayed frequency response overflowed".into());
                }
            }
        }
        Some(base)
    };
    Ok(CtrSpectrum {
        h,
        w,
        rational,
        path_phase: None,
        delay: eval.ctr_delay_seconds(),
        outputs: eval.outputs,
        inputs: eval.inputs,
    })
}
fn ctr_spectrum_dims(s: &CtrSpectrum) -> [usize; 3] {
    if s.inputs == 1 && s.outputs == 1 {
        [s.w.len(), 1, 1]
    } else {
        [s.outputs, s.inputs, s.w.len()]
    }
}

fn ctr_frequency_order<T: Copy>(
    e: &Engine,
    data: Vec<T>,
    nw: usize,
    pairs: usize,
) -> Result<Vec<T>> {
    if pairs == 1 || nw == 1 {
        return Ok(data);
    }
    let mut ordered = Vec::with_capacity(data.len());
    for k in 0..nw {
        e.check_cancel()?;
        ordered.extend((0..pairs).map(|pair| data[k + nw * pair]));
    }
    Ok(ordered)
}

fn ctr_polar(s: &CtrSpectrum) -> (Vec<f64>, Vec<f64>) {
    let raw = s.rational.as_deref().unwrap_or(&s.h);
    let mag = raw.iter().map(|v| v.norm()).collect();
    let phase = s
        .path_phase
        .clone()
        .unwrap_or_else(|| ctr_unwrap_phase(raw, &s.w, s.delay));
    (mag, phase)
}

fn ctr_unwrap_phase(raw: &[C], w: &[f64], delay: f64) -> Vec<f64> {
    let mut phase = vec![0.; raw.len()];
    for (src, dst) in raw.chunks(w.len()).zip(phase.chunks_mut(w.len())) {
        // Unwrap the rational part, then apply the analytic delay phase.
        let mut previous = src[0].arg();
        let mut unwrapped = previous;
        dst[0] = (previous - w[0] * delay) * 180. / PI;
        for k in 1..src.len() {
            let angle = src[k].arg();
            let raw = angle - previous;
            let mut delta = (raw + PI).rem_euclid(2. * PI) - PI;
            if delta == -PI && raw > 0. {
                delta = PI;
            }
            unwrapped += delta;
            dst[k] = if delay == 0. {
                dst[k - 1] + delta * 180. / PI
            } else {
                (unwrapped - w[k] * delay) * 180. / PI
            };
            previous = angle;
        }
    }
    phase
}

#[derive(Clone, Copy)]
struct CtrMargins {
    gm: f64,
    pm: f64,
    wcg: f64,
    wcp: f64,
}
fn ctr_refine(eval: &CtrEvaluator, mut lo: f64, mut hi: f64, phase: bool) -> Result<f64> {
    let criterion = |w: f64| -> Result<f64> {
        if phase {
            Ok(eval.ctr_frequency(w)?[0].im)
        } else {
            Ok(eval.ctr_frequency_base(w)?[0].norm().ln())
        }
    };
    let mut a = criterion(lo)?;
    if a == 0. {
        return Ok(lo);
    }
    if criterion(hi)? == 0. {
        return Ok(hi);
    }
    for _ in 0..60 {
        let mid = if lo > 0. {
            (lo.ln() / 2. + hi.ln() / 2.).exp()
        } else {
            (lo + hi) / 2.
        };
        let f = criterion(mid)?;
        if f == 0. {
            return Ok(mid);
        }
        if a.signum() == f.signum() {
            lo = mid;
            a = f;
        } else {
            hi = mid;
        }
    }
    Ok((lo + hi) / 2.)
}
fn ctr_refine_delayed_phase(
    ev: &CtrEvaluator,
    mut lo: f64,
    mut hi: f64,
    pa: f64,
    pb: f64,
    target: f64,
) -> Result<f64> {
    let increasing = pb > pa;
    if target == pa {
        return Ok(lo);
    }
    if target == pb {
        return Ok(hi);
    }
    let delay = ev.ctr_delay_seconds().to_degrees();
    let base_phase = pa + lo * delay;
    for _ in 0..60 {
        let mid = lo + (hi - lo) / 2.;
        let wrapped = ev.ctr_frequency_base(mid)?[0].arg().to_degrees();
        // Unwrap the base response on its local branch. The known delay
        // can span many turns inside this same bracket.
        let angle =
            base_phase + (wrapped - base_phase + 180.).rem_euclid(360.) - 180. - mid * delay;
        if (angle < target) == increasing {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(lo + (hi - lo) / 2.)
}
/// Every crossing found on the grid, in discovery order: phase crossings of
/// -180 (mod 360) as (frequency, gain margin) and unity-gain crossings as
/// (frequency, phase margin). `endpoint` marks a first gain entry taken at the
/// exact discrete Nyquist frequency. margin keeps the dominant ones; allmargin
/// lists them all.
struct CtrCrossings {
    gain: Vec<(f64, f64)>,
    phase: Vec<(f64, f64)>,
    endpoint: bool,
}
/// For all frequencies >= w, bound a strictly proper |N(jw)/D(jw)| by
/// dividing through by w^deg(D) and applying the triangle inequalities.
/// Once below min(Gm,1/Gm), no later crossing can improve either margin.
fn ctr_below_gain_tail(n: &Matrix, d: &Matrix, w: f64, floor: f64) -> bool {
    if floor == 0. || w <= 0. || n.len() >= d.len() {
        return false;
    }
    let inv = w.recip();
    let bound = |coeffs: &[f64]| coeffs.iter().rev().fold(0., |v, a| v * inv + a.abs());
    let lower = d.data()[0].abs() - inv * bound(&d.data()[1..]);
    let upper = inv.powi((d.len() - n.len()) as i32) * bound(n.data());
    upper < floor * lower
}
fn ctr_crossings(
    e: &Engine,
    mag: &[f64],
    phase: &[f64],
    w: &[f64],
    eval: Option<&CtrEvaluator>,
    gain_tail: Option<&(Matrix, Matrix)>,
) -> Result<CtrCrossings> {
    if w.len() < 2
        || mag.len() != w.len()
        || phase.len() != w.len()
        || mag.iter().any(|x| !x.is_finite() || *x < 0.)
        || phase.iter().any(|x| !x.is_finite())
        || w.iter().any(|x| !x.is_finite() || *x < 0.)
        || w.windows(2).any(|v| v[1] <= v[0])
    {
        return Err(
            "margin requires equal-length magnitude, phase and increasing frequency vectors".into(),
        );
    }
    let mut r = CtrCrossings {
        gain: Vec::new(),
        phase: Vec::new(),
        endpoint: false,
    };
    // Include the discrete Nyquist endpoint independently of phase unwrapping
    // and of the sign of floating-point zero in the complex response.
    if let Some(ev) = eval
        && ev.ts > 0.
        && w[w.len() - 1] == PI / ev.ts
    {
        let h = ev.ctr_frequency(PI / ev.ts)?[0];
        if h.im == 0. && h.re < 0. {
            r.gain.push((PI / ev.ts, -1. / h.re));
            r.endpoint = true;
        }
    }
    let interpolate = |k: usize, f: f64| {
        if w[k] > 0. {
            (w[k].ln() + f * (w[k + 1].ln() - w[k].ln())).exp()
        } else {
            w[k] + f * (w[k + 1] - w[k])
        }
    };
    let mut gain_floor = 0.;
    let analytic = eval.is_some_and(|ev| ev.delay > 0.);
    for k in 0..w.len() - 1 {
        e.check_cancel()?;
        let (pa, pb) = (phase[k], phase[k + 1]);
        let low = ((pa.min(pb) + 180.) / 360.).ceil() as i64;
        let high = ((pa.max(pb) + 180.) / 360.).floor() as i64;
        // Input data are bounded in length and jumps. Reject pathological phase
        // ranges rather than iterating billions of artificial crossings.
        if low <= high
            && let Some((n, d)) = gain_tail
            && ctr_below_gain_tail(n, d, w[k], gain_floor)
        {
            break;
        }
        if !analytic && high.saturating_sub(low) > 4 {
            return Err(
                "margin phase jumps exceed four turns; supply unwrapped sampled phase".into(),
            );
        }
        if pa != pb {
            if analytic
                && low <= high
                && (high.saturating_sub(low) >= (CTR_POINTS - r.gain.len()) as i64
                    || pa.abs().max(pb.abs()) > 360. * (1u64 << 52) as f64)
            {
                return Err("Delayed margin search exceeds resolvable crossing count; use sampled frequency data".into());
            }
            for index in low..=high {
                // Visit crossings in increasing frequency for equal gain margins.
                let turn = if analytic && pb < pa {
                    high - (index - low)
                } else {
                    index
                };
                if turn % 32 == 0 {
                    e.check_cancel()?;
                }
                let f = (-180. + 360. * turn as f64 - pa) / (pb - pa);
                if !(0. ..=1.).contains(&f) {
                    continue;
                }
                let wc = if let Some(ev) = eval {
                    if ev.ctr_longest_delay() == 0. {
                        ctr_refine(ev, w[k], w[k + 1], true)?
                    } else {
                        ctr_refine_delayed_phase(
                            ev,
                            w[k],
                            w[k + 1],
                            pa,
                            pb,
                            -180. + 360. * turn as f64,
                        )?
                    }
                } else {
                    interpolate(k, f)
                };
                let amplitude = if let Some(ev) = eval {
                    let h = ev.ctr_frequency(wc)?[0];
                    if h.re >= 0. {
                        continue;
                    }
                    ev.ctr_frequency_base(wc)?[0].norm()
                } else if mag[k] > 0. && mag[k + 1] > 0. {
                    (mag[k].ln() + f * (mag[k + 1].ln() - mag[k].ln())).exp()
                } else {
                    mag[k] + f * (mag[k + 1] - mag[k])
                };
                let gm = 1. / amplitude;
                gain_floor = gain_floor.max(gm.min(amplitude));
                r.gain.push((wc, gm));
            }
        }
        let (ma, mb) = (mag[k] - 1., mag[k + 1] - 1.);
        if ma == mb || ma * mb > 0. {
            continue;
        }
        let f = if mag[k] > 0. && mag[k + 1] > 0. {
            -mag[k].ln() / (mag[k + 1].ln() - mag[k].ln())
        } else {
            -ma / (mb - ma)
        };
        let wc = if let Some(ev) = eval {
            ctr_refine(ev, w[k], w[k + 1], false)?
        } else {
            interpolate(k, f)
        };
        let angle = if let Some(ev) = eval {
            ev.ctr_frequency(wc)?[0].arg() * 180. / PI
        } else {
            pa + f * (pb - pa)
        };
        r.phase.push((wc, angle.rem_euclid(360.) - 180.));
    }
    Ok(r)
}
/// The dominant margins: the gain margin nearest to 0 dB (smallest |log Gm|)
/// and the phase margin of smallest magnitude; the first found wins ties.
fn ctr_dominant(c: &CtrCrossings) -> CtrMargins {
    let mut r = CtrMargins {
        gm: f64::INFINITY,
        pm: f64::INFINITY,
        wcg: f64::NAN,
        wcp: f64::NAN,
    };
    for (i, &(wc, gm)) in c.gain.iter().enumerate() {
        if (i == 0 && c.endpoint) || gm.ln().abs() < r.gm.ln().abs() {
            r.gm = gm;
            r.wcg = wc;
        }
    }
    for &(wc, pm) in &c.phase {
        if pm.abs() < r.pm.abs() {
            r.pm = pm;
            r.wcp = wc;
        }
    }
    r
}
fn ctr_margins(
    e: &Engine,
    mag: &[f64],
    phase: &[f64],
    w: &[f64],
    eval: Option<&CtrEvaluator>,
) -> Result<CtrMargins> {
    Ok(ctr_dominant(&ctr_crossings(e, mag, phase, w, eval, None)?))
}
fn ctr_margin_system(
    e: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<(CtrSpectrum, CtrMargins)> {
    let (spectrum, crossings) = ctr_margin_crossings(e, args, true, out)?;
    Ok((spectrum, ctr_dominant(&crossings)))
}
/// All crossings of a SISO model on margin's grid (shared by margin and allmargin).
fn ctr_margin_crossings(
    e: &mut Engine,
    args: &[Value],
    dominant: bool,
    out: &mut Execution,
) -> Result<(CtrSpectrum, CtrCrossings)> {
    if args.is_empty() || args.len() > 2 {
        return Err("Expected a SISO system and optional plot frequency vector".into());
    }
    let s = ctr_lti(e, &args[0])?;
    let ev = ctr_evaluator(e, &s, out)?;
    if ev.inputs != 1 || ev.outputs != 1 {
        return Err("margin requires a SISO system".into());
    }
    let w = ctr_frequency_grid(e, &args[0], &s, args.get(1), out)?;
    let coeffs = if s.ts == 0. && args.len() == 1 && !s.has_internal_delays() {
        Some(e.ctl_tf(&s, out)?)
    } else {
        None
    };
    let w = match &coeffs {
        Some(nd) => ctr_margin_grid(e, nd, w)?,
        None => w,
    };
    let w = ctr_delay_margin_grid(
        w,
        ev.ctr_longest_delay(),
        ev.ts,
        args.len() == 1,
        s.has_internal_delays(),
    )?;
    let spectrum = ctr_sample_spectrum(e, &ev, w)?;
    let (mag, phase) = ctr_polar(&spectrum);
    let gain_tail = coeffs.as_ref().filter(|_| dominant && ev.delay > 0.);
    let crossings = ctr_crossings(e, &mag, &phase, &spectrum.w, Some(&ev), gain_tail)?;
    Ok((spectrum, crossings))
}
fn ctr_stepinfo(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err("Use stepinfo(sys[,t]) or stepinfo(y,t)".into());
    }
    let (y, t, final_values, outputs, inputs) = if control_lti::is_lti(&args[0]) {
        let r = ctr_time_response(e, "step", args, false, out)?;
        let Value::Matrix(gain) = e.call("dcgain", vec![args[0].clone()], out)? else {
            return Err("Expected a real DC gain".into());
        };
        (r.y, r.t, gain.data().to_vec(), r.outputs, r.inputs)
    } else {
        if args.len() != 2 {
            return Err("Response data require an explicit time vector".into());
        }
        let y = ctr_real(&args[0])?;
        let t = ctr_vector(&args[1])?;
        if t.len() < 2 || t.windows(2).any(|v| v[1] <= v[0]) {
            return Err("stepinfo time must be increasing with at least two samples".into());
        }
        let y = if y.rows() == 1 && y.cols() == t.len() {
            y.reshape(t.len(), 1)?
        } else {
            y
        };
        if y.rows() != t.len() || y.cols() == 0 {
            return Err("stepinfo needs one row per time and one column per response".into());
        }
        let final_values = (0..y.cols()).map(|j| y.get(t.len() - 1, j)).collect();
        let outputs = y.cols();
        (y, t, final_values, outputs, 1)
    };
    let names = [
        "RiseTime",
        "SettlingTime",
        "Overshoot",
        "Undershoot",
        "Peak",
        "PeakTime",
        "SettlingMin",
        "SettlingMax",
    ];
    let mut slots = Vec::with_capacity(names.len() * y.cols());
    for (j, &yf) in final_values.iter().enumerate() {
        e.check_cancel()?;
        let values = &y.data()[j * t.len()..(j + 1) * t.len()];
        let mut peak_index = 0;
        for k in 1..values.len() {
            if values[k].abs() > values[peak_index].abs() {
                peak_index = k;
            }
        }
        let mut metrics = [f64::NAN; 8];
        metrics[4] = values[peak_index].abs();
        metrics[5] = t[peak_index];
        if yf.is_finite() && yf != 0. {
            let sign = yf.signum();
            let low = values.iter().position(|v| sign * v >= 0.1 * yf.abs());
            let high = values.iter().position(|v| sign * v >= 0.9 * yf.abs());
            if let (Some(low), Some(high)) = (low, high) {
                metrics[0] = t[high] - t[low];
                metrics[6] = values[high..].iter().copied().fold(yf, f64::min);
                metrics[7] = values[high..].iter().copied().fold(yf, f64::max);
            }
            metrics[1] = match values.iter().rposition(|v| (v / yf - 1.).abs() >= 0.02) {
                Some(k) if k + 1 < t.len() => t[k + 1],
                None => t[0],
                _ => f64::NAN,
            };
            let max = values
                .iter()
                .map(|v| sign * v)
                .fold(f64::NEG_INFINITY, f64::max);
            let min = values
                .iter()
                .map(|v| sign * v)
                .fold(f64::INFINITY, f64::min);
            metrics[2] = (max / yf.abs() - 1.).max(0.) * 100.;
            metrics[3] = (-min / yf.abs()).max(0.) * 100.;
        }
        slots.extend(
            metrics
                .into_iter()
                .map(|x| Arc::new(Array::Numeric(Matrix::scalar(x)))),
        );
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        outputs,
        inputs,
        names.iter().map(|s| (*s).into()).collect(),
        slots,
        &e.cancel,
    )?)))
}

enum CtrPlot {
    Time(CtrTime),
    Frequency(CtrSpectrum, Option<CtrMargins>),
}
/// The subplot cell a rows-by-cols response grid nests in. A response plot
/// draws in the current axes' place (as MATLAB's do): when `subplot` has just
/// selected an empty cell of a figure split into cells, the grid's panels
/// split that cell instead of laying out the whole figure, so the other cells
/// stay. Any other state (no cells, a tiled layout, hold on, axes that already
/// show data, as after an earlier response plot) keeps the whole-figure grid.
fn ctr_host_cell(e: &Engine, rows: usize, cols: usize) -> Option<crate::plot::Position> {
    let f = e.figure.as_ref()?;
    let layout = f.layout.as_ref()?;
    let p = layout.active;
    let fresh = layout.tiled.is_none() && !f.hold && f.axes.series.is_empty();
    (rows * cols > 1
        && fresh
        && p.custom.is_none()
        && p != crate::plot::Position::default()
        && usize::from(p.rows) * rows <= 16
        && usize::from(p.columns) * cols <= 16)
        .then_some(p)
}
fn ctr_select_axes(
    e: &mut Engine,
    rows: usize,
    cols: usize,
    index: usize,
    host: Option<crate::plot::Position>,
    out: &mut Execution,
) -> Result<Value> {
    if rows * cols > 1 {
        let scalar = |v: usize| Value::Matrix(Matrix::scalar(v as f64));
        let args = match host {
            // Panel (i, j) of the grid inside the host cell, on the host's
            // grid refined rows-by-cols times.
            Some(p) => {
                let (i, j) = ((index - 1) / cols, (index - 1) % cols);
                let (h, w) = (usize::from(p.bottom - p.top), usize::from(p.right - p.left));
                let columns = usize::from(p.columns) * cols;
                let top = usize::from(p.top) * rows + i * h;
                let left = usize::from(p.left) * cols + j * w;
                let cells: Vec<f64> = (top..top + h)
                    .flat_map(|r| (left..left + w).map(move |c| (r * columns + c + 1) as f64))
                    .collect();
                vec![
                    scalar(usize::from(p.rows) * rows),
                    scalar(columns),
                    Value::Matrix(Matrix::from_column_vec(1, cells.len(), cells)?),
                ]
            }
            None => vec![scalar(rows), scalar(cols), scalar(index)],
        };
        e.call("subplot", args, out)?;
    }
    e.call("gca", vec![], out)
}
fn ctr_draw_line(
    e: &mut Engine,
    ax: &Value,
    x: &[f64],
    y: Vec<f64>,
    log: bool,
    style: Option<&str>,
    out: &mut Execution,
) -> Result<()> {
    let mut args = vec![ax.clone(), ctr_column(x.to_vec())?, ctr_column(y)?];
    if let Some(style) = style {
        args.push(Value::Text(style.into()));
    }
    e.call(if log { "semilogx" } else { "plot" }, args, out)?;
    Ok(())
}
fn ctr_hold(e: &mut Engine, ax: &Value, hold: bool, out: &mut Execution) -> Result<()> {
    e.call(
        "hold",
        vec![
            ax.clone(),
            Value::Text(if hold { "on" } else { "off" }.into()),
        ],
        out,
    )?;
    Ok(())
}
fn ctr_plot_inner(
    e: &mut Engine,
    name: &str,
    data: &[CtrPlot],
    labels: Option<&(Vec<String>, Vec<String>)>,
    out: &mut Execution,
) -> Result<()> {
    let (p, m) = match &data[0] {
        CtrPlot::Time(r) => (r.outputs, r.inputs),
        CtrPlot::Frequency(r, _) => (r.outputs, r.inputs),
    };
    let bode = matches!(name, "bode" | "margin");
    let rows = if bode { 2 * p } else { p };
    if rows > 16 || m > 16 || data.len() > 16 {
        return Err("Response plot exceeds 16 rows, columns or overlaid systems".into());
    }
    let polar: Vec<_> = data
        .iter()
        .map(|item| {
            e.check_cancel()?;
            Ok(match item {
                CtrPlot::Frequency(r, _) if bode => Some(ctr_polar(r)),
                _ => None,
            })
        })
        .collect::<Result<_>>()?;
    let host = ctr_host_cell(e, rows, m);
    let mut time_axes = Vec::new();
    for i in 0..p {
        for j in 0..m {
            for panel in 0..if bode { 2 } else { 1 } {
                let ax = ctr_select_axes(
                    e,
                    rows,
                    m,
                    (i * if bode { 2 } else { 1 } + panel) * m + j + 1,
                    host,
                    out,
                )?;
                if p * m > 1
                    && matches!(name, "step" | "impulse")
                    && let Value::Container(Array::Graphics(handles)) = &ax
                {
                    time_axes.push(handles.get_linear(0)?.object.clone());
                }
                let old_hold = e.figure.as_ref().is_some_and(|f| f.hold);
                for (system, item) in data.iter().enumerate() {
                    if system > 0 {
                        ctr_hold(e, &ax, true, out)?;
                    }
                    let trace = i + p * j;
                    match item {
                        CtrPlot::Time(r) => ctr_draw_line(
                            e,
                            &ax,
                            &r.t,
                            r.y.data()[trace * r.t.len()..(trace + 1) * r.t.len()].to_vec(),
                            false,
                            None,
                            out,
                        )?,
                        CtrPlot::Frequency(r, _) => {
                            if bode {
                                let (mag, phase) = polar[system].as_ref().unwrap();
                                let y = if panel == 0 {
                                    mag[trace * r.w.len()..(trace + 1) * r.w.len()]
                                        .iter()
                                        .map(|v| 20. * v.log10())
                                        .collect()
                                } else {
                                    phase[trace * r.w.len()..(trace + 1) * r.w.len()].to_vec()
                                };
                                ctr_draw_line(e, &ax, &r.w, y, true, None, out)?;
                            } else {
                                let h = &r.h[trace * r.w.len()..(trace + 1) * r.w.len()];
                                let re: Vec<f64> = h.iter().map(|v| v.re).collect();
                                ctr_draw_line(
                                    e,
                                    &ax,
                                    &re,
                                    h.iter().map(|v| v.im).collect(),
                                    false,
                                    None,
                                    out,
                                )?;
                                ctr_hold(e, &ax, true, out)?;
                                ctr_draw_line(
                                    e,
                                    &ax,
                                    &re,
                                    h.iter().map(|v| -v.im).collect(),
                                    false,
                                    Some("--"),
                                    out,
                                )?;
                            }
                        }
                    }
                }
                if name == "nyquist" {
                    ctr_hold(e, &ax, true, out)?;
                    ctr_draw_line(e, &ax, &[-1.], vec![0.], false, Some("x"), out)?;
                }
                ctr_hold(e, &ax, old_hold, out)?;
                let (x, y) = if bode {
                    (
                        "Frequency (rad / time unit)",
                        if panel == 0 {
                            "Magnitude (dB)"
                        } else {
                            "Phase (deg)"
                        },
                    )
                } else if name == "nyquist" {
                    ("Real", "Imaginary")
                } else {
                    ("Time", "Amplitude")
                };
                e.call("xlabel", vec![ax.clone(), Value::Text(x.into())], out)?;
                let ylabel = match labels {
                    Some((_, outputs)) if j == 0 && panel == 0 => {
                        format!("To: {}\n{y}", outputs[i])
                    }
                    _ => y.into(),
                };
                e.call("ylabel", vec![ax.clone(), Value::Text(ylabel)], out)?;
                if let Some((inputs, _)) = labels {
                    if i == 0 && panel == 0 {
                        e.call(
                            "title",
                            vec![ax, Value::Text(format!("From: {}", inputs[j]))],
                            out,
                        )?;
                    }
                } else if p * m > 1 {
                    e.call(
                        "title",
                        vec![
                            ax,
                            Value::Text(format!("Output {}, input {}", i + 1, j + 1)),
                        ],
                        out,
                    )?;
                }
            }
        }
    }
    if time_axes.len() > 1 {
        if let Some(figure) = e.figure.as_mut() {
            figure.set_linked_axes(&time_axes, true, false);
        }
        super::graphics::set_engine_linked_axes(e, &time_axes, true, false)?;
    }
    if name == "margin" {
        let title = data
            .iter()
            .filter_map(|d| {
                if let CtrPlot::Frequency(_, Some(r)) = d {
                    Some(format!(
                        "Gm={:.6}, Pm={:.6} deg; Wcg={:.6}, Wcp={:.6}",
                        r.gm, r.pm, r.wcg, r.wcp
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" | ");
        e.call("sgtitle", vec![Value::Text(title)], out)?;
    }
    Ok(())
}
fn ctr_plot(e: &mut Engine, name: &str, args: &[Value], out: &mut Execution) -> Result<()> {
    if name == "margin" && args.len() == 3 && !control_lti::is_lti(&args[0]) {
        let mag = ctr_vector(&args[0])?;
        let phase = ctr_vector(&args[1])?;
        let w = ctr_vector(&args[2])?;
        let margins = ctr_margins(e, &mag, &phase, &w, None)?;
        let spectrum = CtrSpectrum {
            h: mag
                .iter()
                .zip(&phase)
                .map(|(&m, &p)| C::from_polar(m, p * PI / 180.))
                .collect(),
            w,
            rational: None,
            path_phase: None,
            delay: 0.,
            outputs: 1,
            inputs: 1,
        };
        return ctr_render_plot(
            e,
            name,
            &[CtrPlot::Frequency(spectrum, Some(margins))],
            None,
            out,
        );
    }
    let systems = args.iter().take_while(|v| control_lti::is_lti(v)).count();
    if systems == 0 {
        return Err("Response plots require an LTI system".into());
    }
    if systems > 16 {
        return Err("At most 16 systems may be overlaid".into());
    }
    let mut data = Vec::with_capacity(systems);
    let mut shape = None;
    let mut labels = None;
    let mut entries = 0usize;
    for s in &args[..systems] {
        let model = ctr_lti(e, s)?;
        let (inputs, outputs) = model.ports();
        let io = (
            outputs,
            if matches!(name, "initial" | "lsim") {
                1
            } else {
                inputs
            },
        );
        if shape.is_none() && io.0 * io.1 > 1 && matches!(name, "step" | "impulse" | "bode") {
            labels = Some(model.response_port_labels());
        }
        let rows = io.0
            * if matches!(name, "bode" | "margin") {
                2
            } else {
                1
            };
        if rows > 16 || io.1 > 16 {
            return Err("Response plot exceeds 16 rows or columns".into());
        }
        if shape.is_some_and(|v| v != io) {
            return Err("Overlaid systems must have the same input/output dimensions".into());
        }
        shape = Some(io);
        let mut a = vec![s.clone()];
        a.extend_from_slice(&args[systems..]);
        let item = if matches!(name, "bode" | "nyquist" | "margin") {
            if name == "margin" {
                let (r, m) = ctr_margin_system(e, &a, out)?;
                CtrPlot::Frequency(r, Some(m))
            } else {
                if a.len() > 2 {
                    return Err("Expected a system and an optional frequency vector".into());
                }
                CtrPlot::Frequency(ctr_spectrum(e, s, a.get(1), name == "bode", out)?, None)
            }
        } else {
            CtrPlot::Time(ctr_time_response(e, name, &a, false, out)?)
        };
        entries += match &item {
            CtrPlot::Time(r) => r.y.len() + r.x.len() + r.t.len(),
            CtrPlot::Frequency(r, _) => 2 * r.h.len() + r.w.len(),
        };
        if entries > CTR_ENTRIES {
            return Err("Overlaid response data exceed 32 MiB".into());
        }
        data.push(item);
    }
    ctr_render_plot(e, name, &data, labels.as_ref(), out)
}
fn ctr_render_plot(
    e: &mut Engine,
    name: &str,
    data: &[CtrPlot],
    labels: Option<&(Vec<String>, Vec<String>)>,
    out: &mut Execution,
) -> Result<()> {
    // Restore all graphics selections and immutable snapshots if any draw fails.
    let old = (
        e.current_figure,
        e.figure.clone(),
        e.other_figures.clone(),
        e.linked_axes.clone(),
    );
    let result = ctr_plot_inner(e, name, data, labels, out);
    if result.is_err() {
        (e.current_figure, e.figure, e.other_figures, e.linked_axes) = old;
    }
    result
}
// pzmap: poles ('x') and zeros ('o') from the existing pole/zero builtins,
// drawn with the existing plot builtin on the current (or given) axes.
fn ctr_pzmap(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let skip = usize::from(args.first().is_some_and(|v| {
        matches!(v, Value::Container(crate::Array::Graphics(_))) && !control_lti::is_lti(v)
    }));
    let systems = &args[skip..];
    if systems.is_empty() || !systems.iter().all(control_lti::is_lti) {
        return Err("pzmap expects pzmap(sys1, sys2, ...), optionally after an axes handle".into());
    }
    if systems.len() > 16 {
        return Err("pzmap: at most 16 systems may be overlaid".into());
    }
    let mut roots = Vec::with_capacity(systems.len());
    for s in systems {
        let model = ctr_lti(e, s)?;
        model.require_siso("pzmap")?;
        let p = e.call("pole", vec![s.clone()], out)?;
        let z = e.call("zero", vec![s.clone()], out)?;
        roots.push((p, z, model.ts != 0.));
    }
    if count > 0 {
        if count > 2 || systems.len() != 1 || skip == 1 {
            return Err("Use [p, z] = pzmap(sys) with one system".into());
        }
        let (p, z, _) = roots.remove(0);
        let mut v = vec![p, z];
        v.truncate(count);
        e.charge_function_outputs(&v)?;
        return Ok(v);
    }
    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let result = ctr_pzmap_draw(
        e,
        args.get(..skip).and_then(|a| a.first()),
        &roots,
        ["Pole-Zero Map", "Real Axis", "Imaginary Axis"],
        out,
    );
    if result.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    result.map(|()| vec![])
}
fn ctr_pzmap_parts(v: &Value) -> Result<(Vec<f64>, Vec<f64>)> {
    Ok(match v {
        Value::Matrix(m) => (m.data().to_vec(), vec![0.; m.len()]),
        Value::Complex(m) => (
            m.data().iter().map(|z| z.re).collect(),
            m.data().iter().map(|z| z.im).collect(),
        ),
        _ => return Err("pzmap: pole/zero returned a non-numeric value".into()),
    })
}
fn ctr_pzmap_draw(
    e: &mut Engine,
    ax: Option<&Value>,
    roots: &[(Value, Value, bool)],
    [title, xlabel, ylabel]: [&str; 3],
    out: &mut Execution,
) -> Result<()> {
    let ax = match ax {
        Some(ax) => ax.clone(),
        None => e.call("gca", vec![], out)?,
    };
    let old_hold = e.figure.as_ref().is_some_and(|f| f.hold);
    let mut extent = 0_f64;
    let mut discrete = false;
    let mut first = true;
    for (p, z, d) in roots {
        discrete |= *d;
        for (v, marker) in [(p, "x"), (z, "o")] {
            let (re, im) = ctr_pzmap_parts(v)?;
            if re.is_empty() {
                continue;
            }
            extent = re
                .iter()
                .chain(&im)
                .filter(|x| x.is_finite())
                .fold(extent, |a, x| a.max(x.abs()));
            if !first {
                ctr_hold(e, &ax, true, out)?;
            }
            first = false;
            ctr_draw_line(e, &ax, &re, im, false, Some(marker), out)?;
        }
    }
    if discrete {
        extent = extent.max(1.);
    }
    let extent = if extent > 0. { 1.2 * extent } else { 1. };
    if !first {
        ctr_hold(e, &ax, true, out)?;
    }
    // Dashed real and imaginary axes, then the dotted unit circle (discrete).
    ctr_draw_line(
        e,
        &ax,
        &[-extent, extent],
        vec![0., 0.],
        false,
        Some("k--"),
        out,
    )?;
    ctr_hold(e, &ax, true, out)?;
    ctr_draw_line(
        e,
        &ax,
        &[0., 0.],
        vec![-extent, extent],
        false,
        Some("k--"),
        out,
    )?;
    if discrete {
        let n = 361;
        let angle = |k: usize| 2. * PI * k as f64 / (n - 1) as f64;
        let x: Vec<f64> = (0..n).map(|k| libm::cos(angle(k))).collect();
        let y = (0..n).map(|k| libm::sin(angle(k))).collect();
        ctr_draw_line(e, &ax, &x, y, false, Some("k:"), out)?;
    }
    ctr_hold(e, &ax, old_hold, out)?;
    if !title.is_empty() {
        e.call("title", vec![ax.clone(), Value::Text(title.into())], out)?;
    }
    e.call("xlabel", vec![ax.clone(), Value::Text(xlabel.into())], out)?;
    e.call("ylabel", vec![ax, Value::Text(ylabel.into())], out)?;
    Ok(())
}
/// zplane(z, p) with column vectors of zeros and poles, or zplane(b, a) with
/// row vectors of coefficients in powers of z^-1 (the shorter one padded with
/// trailing zeros, which puts the extra roots at the origin).
fn ctr_zplane(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > 0 {
        return Err(
            "zplane: output handles are not supported yet; call zplane as a statement".into(),
        );
    }
    if args.is_empty() || args.len() > 2 {
        return Err(
            "Use zplane(z, p) with column vectors of zeros and poles or zplane(b, a) with row vectors of coefficients"
                .into(),
        );
    }
    let row = |v: &Value| match v {
        Value::Matrix(m) => m.rows() <= 1,
        Value::Complex(m) => m.rows() <= 1,
        _ => false,
    };
    // An L-by-6 matrix with L >= 2 is second-order sections (a single row is b).
    let sos = match args {
        [Value::Matrix(m)] if m.cols() == 6 && m.rows() >= 2 => Some(m),
        _ => None,
    };
    let (z, p) = if let Some(sos) = sos {
        let zpk = crate::dsp::iir::sos2zpk(sos).map_err(|err| format!("zplane: {err}"))?;
        let column = |roots: Vec<C>| -> Result<Value> {
            Ok(Value::Complex(ComplexMatrix::from_column_vec(
                roots.len(),
                1,
                roots,
                &e.cancel,
            )?))
        };
        (column(zpk.zeros)?, column(zpk.poles)?)
    } else if args.iter().all(row) {
        let b = ctr_real(&args[0])
            .map_err(|_| "zplane: coefficients must be finite real row vectors")?;
        let a = match args.get(1) {
            Some(v) => {
                ctr_real(v).map_err(|_| "zplane: coefficients must be finite real row vectors")?
            }
            None => Matrix::scalar(1.),
        };
        let n = b.len().max(a.len());
        let padded = |m: &Matrix| -> Result<Value> {
            let mut d = m.data().to_vec();
            d.resize(n, 0.);
            Ok(Value::Matrix(Matrix::from_column_vec(1, n, d)?))
        };
        let z = e.call("roots", vec![padded(&b)?], out)?;
        let p = e.call("roots", vec![padded(&a)?], out)?;
        (z, p)
    } else {
        let p = args
            .get(1)
            .cloned()
            .unwrap_or(Value::Matrix(Matrix::from_column_vec(0, 1, vec![])?));
        (args[0].clone(), p)
    };
    for v in [&z, &p] {
        let (re, im) = ctr_pzmap_parts(v).map_err(|_| "zplane: zeros and poles must be numeric")?;
        if re.iter().chain(&im).any(|x| !x.is_finite()) {
            return Err("zplane: zeros and poles must be finite".into());
        }
    }
    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let result = ctr_pzmap_draw(
        e,
        None,
        &[(p, z, true)],
        ["", "Real Part", "Imaginary Part"],
        out,
    )
    .and_then(|()| {
        e.call("axis", vec![Value::Text("equal".into())], out)
            .map(drop)
    });
    if result.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    result.map(|()| vec![])
}
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if name == "pzmap" {
        return ctr_pzmap(e, args, count, out);
    }
    if name == "zplane" {
        return ctr_zplane(e, args, count, out);
    }
    if matches!(name, "nichols" | "sigma" | "allmargin" | "ngrid") {
        return freq::call(e, name, args, count, out);
    }
    if count == 0 && plots(name) {
        ctr_plot(e, name, args, out)?;
        return Ok(vec![]);
    }
    let max = match name {
        "lsim" | "step" | "impulse" | "initial" | "bode" | "nyquist" => 3,
        "margin" => 4,
        _ => 1,
    };
    if count == 0 || count > max {
        return Err(format!("{name} supports 1..{max} outputs"));
    }
    let mut results = match name {
        "step" | "impulse" | "initial" | "lsim" => {
            let r = ctr_time_response(e, name, args, count == 3, out)?;
            let nt = r.t.len();
            let y = r.y.with_dims(&[nt, r.outputs, r.inputs])?;
            let mut v = vec![Value::Matrix(y), ctr_column(r.t)?];
            if count == 3 {
                v.push(Value::Matrix(r.x.with_dims(&[
                    nt,
                    r.x.cols() / r.inputs,
                    r.inputs,
                ])?));
            }
            v
        }
        "stepinfo" => vec![ctr_stepinfo(e, args, out)?],
        "bode" | "nyquist" | "freqresp" => {
            if name == "freqresp" && args.len() != 2 {
                return Err("Use freqresp(sys,w)".into());
            }
            if args.is_empty() || args.len() > 2 {
                return Err("Expected a system and an optional frequency vector".into());
            }
            let r = ctr_spectrum(e, &args[0], args.get(1), name == "bode" && count > 1, out)?;
            if name == "freqresp" {
                let [rows, cols, pages] = ctr_spectrum_dims(&r);
                let h = ctr_frequency_order(e, r.h, r.w.len(), r.inputs * r.outputs)?;
                vec![Value::Complex(
                    ComplexMatrix::from_column_vec(rows, cols * pages, h, &e.cancel)?
                        .with_dims(&[rows, cols, pages])?,
                )]
            } else {
                let (a, b) = if name == "bode" {
                    ctr_polar(&r)
                } else {
                    (
                        r.h.iter().map(|v| v.re).collect(),
                        r.h.iter().map(|v| v.im).collect(),
                    )
                };
                let [rows, cols, pages] = ctr_spectrum_dims(&r);
                let pack = |data| {
                    let data = ctr_frequency_order(e, data, r.w.len(), r.inputs * r.outputs)?;
                    Matrix::from_column_vec(rows, cols * pages, data)?
                        .with_dims(&[rows, cols, pages])
                };
                vec![
                    Value::Matrix(pack(a)?),
                    Value::Matrix(pack(b)?),
                    ctr_column(r.w)?,
                ]
            }
        }
        "evalfr" => {
            if args.len() != 2 {
                return Err("Use evalfr(sys,s) (a z-plane point for discrete systems)".into());
            }
            let s = ctr_lti(e, &args[0])?;
            let z = match &args[1] {
                Value::Matrix(m) => C::new(m.as_scalar()?, 0.),
                Value::Complex(m) if m.len() == 1 => m.data()[0],
                _ => return Err("evalfr point must be a real or complex scalar".into()),
            };
            if !z.re.is_finite() || !z.im.is_finite() {
                return Err("evalfr point must be finite".into());
            }
            vec![if s.rational_grid() {
                control_lti::evalfr(e, &s, z, out)?
            } else {
                let ev = ctr_evaluator(e, &s, out)?;
                Value::Complex(ComplexMatrix::from_column_vec(
                    ev.outputs,
                    ev.inputs,
                    ev.ctr_at(z)?,
                    &e.cancel,
                )?)
            }]
        }
        "margin" => {
            let r = if args.len() == 1 {
                ctr_margin_system(e, args, out)?.1
            } else if args.len() == 3 {
                ctr_margins(
                    e,
                    &ctr_vector(&args[0])?,
                    &ctr_vector(&args[1])?,
                    &ctr_vector(&args[2])?,
                    None,
                )?
            } else {
                return Err("Use margin(sys) or margin(mag,phase,w)".into());
            };
            vec![r.gm, r.pm, r.wcg, r.wcp]
                .into_iter()
                .map(|x| Value::Matrix(Matrix::scalar(x)))
                .collect()
        }
        _ => return Err("Unknown control response function".into()),
    };
    results.truncate(count);
    e.charge_function_outputs(&results)?;
    e.check_cancel()?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_port_frequency_evaluator_rejects_before_parallel_spectrum() {
        for workers in [1, 4] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    for (inputs, outputs) in [(0, 1), (1, 0), (0, 0)] {
                        // The public ss constructor already rejects zero ports.
                        // Build directly to guard the response boundary as well:
                        // n=40 and 1000 samples would enter parallel chunking.
                        let mut e = Engine::default();
                        let mut out = Execution::default();
                        let value = e
                            .call(
                                "ss",
                                vec![
                                    Value::Matrix(Matrix::scalar(-1.)),
                                    Value::Matrix(Matrix::scalar(1.)),
                                    Value::Matrix(Matrix::scalar(1.)),
                                    Value::Matrix(Matrix::scalar(0.)),
                                ],
                                &mut out,
                            )
                            .unwrap();
                        let mut s = ctr_lti(&e, &value).unwrap();
                        s.model = Model::Ss(
                            Matrix::from_column_slice(
                                40,
                                40,
                                DMatrix::<f64>::identity(40, 40).as_slice(),
                            )
                            .unwrap(),
                            Matrix::filled(40, inputs, 0.).unwrap(),
                            Matrix::filled(outputs, 40, 0.).unwrap(),
                            Matrix::filled(outputs, inputs, 0.).unwrap(),
                        );
                        let err = match ctr_evaluator(&mut e, &s, &mut out)
                            .and_then(|ev| ctr_sample_spectrum(&e, &ev, vec![1.; 1000]))
                        {
                            Err(err) => err,
                            Ok(_) => panic!("Zero-port frequency model was accepted"),
                        };
                        assert!(err.contains("at least one input and one output"), "{err}");
                    }
                });
        }
    }
}
