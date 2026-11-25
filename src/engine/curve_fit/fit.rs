//! fit(x, y, model, ...): data checks, exclusion, weights and normalization;
//! linear least squares by QR, nonlinear least squares by the bounded
//! Levenberg-Marquardt core, robust (LAR / bisquare) reweighting, interpolants,
//! and the gof and output structs.
use super::super::{Engine, Execution, Value};
use super::Fitted;
use super::model::{self, FitType, Interp, Kind, Knots};
use super::options::{Display, Method, Options, Robust, text_of};
use super::solve::{self, Problem, Settings};
use super::start;
use crate::{Array, Matrix, Result};
use nalgebra::DMatrix;

/// Rows of data a fit accepts (keeps QR work and Jacobians bounded).
const MAX_POINTS: usize = 2_000_000;
/// Robust reweighting rounds.
const ROBUST_ROUNDS: usize = 100;

pub(super) fn real_matrix(e: &Engine, v: &Value, who: &str, what: &str) -> Result<Matrix> {
    match v {
        Value::Matrix(m) => Ok(m.clone()),
        Value::Logical(l) => l.to_numeric(&e.cancel),
        Value::Container(Array::Typed(a)) => a.to_matrix(),
        Value::Complex(_) => Err(format!("{who}: complex {what} is not supported")),
        _ => Err(format!("{who}: {what} must be real numeric values")),
    }
}

/// The model argument of fit: a fittype or fit object, text, a cell array of
/// linear terms, or an anonymous function.
fn model_arg(e: &mut Engine, v: &Value, out: &mut Execution) -> Result<FitType> {
    if let Some(obj) = super::object(e, v) {
        return Ok(obj.ft().clone());
    }
    model::from_value(e, v, &Default::default(), out)
}

fn problem_values(e: &Engine, v: &Value) -> Result<Vec<f64>> {
    let scalar = |v: &Value| -> Result<f64> {
        let m = real_matrix(e, v, "fit", "a problem parameter value")?;
        if m.len() != 1 {
            return Err("fit: problem parameter values must be scalars".into());
        }
        Ok(m.data()[0])
    };
    match v {
        Value::Container(Array::Cell(c)) => (0..c.len())
            .map(|i| scalar(&Value::from(c.get_linear(i)?.as_ref().clone())))
            .collect(),
        _ => Ok(real_matrix(e, v, "fit", "the problem values")?
            .data()
            .to_vec()),
    }
}

pub(super) fn warn(e: &mut Engine, id: &str, message: &str, out: &mut Execution) -> Result<()> {
    let args = [id, "%s", message].map(|s| Value::Text(s.into())).to_vec();
    e.call("warning", args, out).map(drop)
}

/// Prepared data: included points only, independent columns normalized.
struct Data {
    xs: Vec<Vec<f64>>,
    y: Vec<f64>,
    w: Vec<f64>,
    norm: Vec<(f64, f64)>,
    range: Vec<(f64, f64)>,
}
impl Data {
    fn refs(&self) -> Vec<&[f64]> {
        self.xs.iter().map(Vec::as_slice).collect()
    }
}

fn prepare(e: &Engine, ft: &FitType, xv: &Value, yv: &Value, opts: &Options) -> Result<Data> {
    let x = real_matrix(e, xv, "fit", "X")?;
    let y = real_matrix(e, yv, "fit", "Y")?;
    if y.rows() != 1 && y.cols() != 1 {
        return Err("fit: Y must be a column vector.".into());
    }
    let n = y.len();
    if n > MAX_POINTS {
        return Err(format!(
            "fit: at most {MAX_POINTS} data points are supported"
        ));
    }
    let k = ft.indep.len();
    let columns: Vec<Vec<f64>> = if k == 1 {
        if x.rows() != 1 && x.cols() != 1 {
            return Err(format!(
                "fit: X must have one column for a curve model; this model has the independent variable {}.",
                ft.indep[0]
            ));
        }
        if x.len() != n {
            return Err("fit: X and Y must have the same number of rows.".into());
        }
        vec![x.data().to_vec()]
    } else {
        if x.cols() != 2 {
            return Err(
                "fit: X must be a matrix with two columns for a surface model (fit([x y], z, ...)).".into(),
            );
        }
        if x.rows() != n {
            return Err("fit: X and Y must have the same number of rows.".into());
        }
        vec![x.data()[..n].to_vec(), x.data()[n..].to_vec()]
    };
    if !opts.weights.is_empty() && opts.weights.len() != n {
        return Err(format!(
            "fit: Weights must have one value per data point ({n}), not {}.",
            opts.weights.len()
        ));
    }
    let mut keep = vec![true; n];
    let ex = opts.exclude.data();
    if opts.exclude_logical {
        if !ex.is_empty() && ex.len() != n {
            return Err(format!(
                "fit: a logical Exclude vector must have one value per data point ({n})."
            ));
        }
        for (i, v) in ex.iter().enumerate() {
            keep[i] = *v == 0.;
        }
    } else {
        for &v in ex {
            if v.fract() != 0. || v < 1. || v > n as f64 {
                return Err(format!(
                    "fit: Exclude indices must be integers from 1 to the number of data points ({n})."
                ));
            }
            keep[v as usize - 1] = false;
        }
    }
    let idx: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
    for &i in &idx {
        if !y.data()[i].is_finite() || columns.iter().any(|c| !c[i].is_finite()) {
            return Err(
                "fit: X and Y must not contain NaN or Inf values; remove them or leave them out with 'Exclude'.".into(),
            );
        }
    }
    let mut xs: Vec<Vec<f64>> = columns
        .iter()
        .map(|c| idx.iter().map(|&i| c[i]).collect())
        .collect();
    let range = xs
        .iter()
        .map(|c| {
            c.iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
                    (lo.min(v), hi.max(v))
                })
        })
        .collect();
    let mut norm = Vec::new();
    if opts.normalize {
        for c in &mut xs {
            let m = c.len() as f64;
            let mean = c.iter().sum::<f64>() / m;
            let var = c.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (m - 1.).max(1.);
            let sd = if var > 0. && var.is_finite() {
                var.sqrt()
            } else {
                1.
            };
            for v in c.iter_mut() {
                *v = (*v - mean) / sd;
            }
            norm.push((mean, sd));
        }
    }
    let w = if opts.weights.is_empty() {
        vec![1.; idx.len()]
    } else {
        idx.iter().map(|&i| opts.weights[i]).collect()
    };
    Ok(Data {
        xs,
        y: idx.iter().map(|&i| y.data()[i]).collect(),
        w,
        norm,
        range,
    })
}

/// Bounds of length p (empty means unbounded), checked.
fn bounds(opts: &Options, p: usize) -> Result<(Vec<f64>, Vec<f64>)> {
    let get = |v: &[f64], fill: f64, what: &str| -> Result<Vec<f64>> {
        if v.is_empty() {
            Ok(vec![fill; p])
        } else if v.len() != p {
            Err(format!(
                "fit: {what} must have one value per coefficient ({p}), not {}.",
                v.len()
            ))
        } else {
            Ok(v.to_vec())
        }
    };
    let lower = get(&opts.lower, f64::NEG_INFINITY, "Lower")?;
    let upper = get(&opts.upper, f64::INFINITY, "Upper")?;
    if lower.iter().zip(&upper).any(|(l, u)| l > u) {
        return Err("fit: Lower bounds must be less than or equal to Upper bounds.".into());
    }
    if opts.diff_min > opts.diff_max {
        return Err("fit: DiffMinChange must be less than or equal to DiffMaxChange.".into());
    }
    Ok((lower, upper))
}

/// The weighted problem the nonlinear core iterates on.
struct Nls<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    ft: &'a FitType,
    problem: &'a [f64],
    xs: Vec<&'a [f64]>,
    y: &'a [f64],
    sw: Vec<f64>,
    lower: Vec<f64>,
    upper: Vec<f64>,
    diff: (f64, f64),
}
impl Problem for Nls<'_> {
    fn residual(&mut self, b: &[f64]) -> Result<Vec<f64>> {
        let f = model::values(self.e, self.ft, b, self.problem, None, &self.xs, self.out)?;
        Ok(f.iter()
            .zip(self.y)
            .zip(&self.sw)
            .map(|((f, y), s)| s * (f - y))
            .collect())
    }
    fn jacobian(&mut self, b: &[f64]) -> Result<(DMatrix<f64>, usize)> {
        let (mut j, count) = model::coefficient_jacobian(
            self.e,
            self.ft,
            b,
            self.problem,
            &self.xs,
            Some((&self.lower, &self.upper)),
            self.diff,
            self.out,
        )?;
        for (i, s) in self.sw.iter().enumerate() {
            j.row_mut(i).scale_mut(*s);
        }
        Ok((j, count))
    }
    fn check(&mut self) -> Result<()> {
        self.e.check_cancel()
    }
    fn log(&mut self, text: String) -> Result<()> {
        self.e.emit(self.out, text + "\n")
    }
}

struct Solution {
    coef: Vec<f64>,
    /// Weights of the last (robust) round: user weights times robust weights.
    final_w: Vec<f64>,
    iterations: usize,
    func_count: usize,
    exitflag: i32,
    firstorder: f64,
    algorithm: &'static str,
    message: String,
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.
    }
}

/// Robust weights from residuals and leverages: adjusted residuals
/// r/sqrt(1-h), robust scale MAD/0.6745 (MAD about the median), bisquare
/// (1-u^2)^2 with u = r_adj/(4.685 s), or LAR weights 1/|r_adj|.
fn robust_weights(kind: Robust, r: &[f64], h: &[f64]) -> Option<Vec<f64>> {
    let adj: Vec<f64> = r
        .iter()
        .zip(h)
        .map(|(r, h)| r / (1. - h.min(1. - 1e-12)).sqrt())
        .collect();
    let mut tmp = adj.clone();
    let med = median(&mut tmp);
    let mut dev: Vec<f64> = adj.iter().map(|a| (a - med).abs()).collect();
    let s = median(&mut dev) / 0.6745;
    if !s.is_finite() || s <= 0. {
        return None;
    }
    Some(match kind {
        Robust::Bisquare => adj
            .iter()
            .map(|a| {
                let u = a / (4.685 * s);
                if u.abs() < 1. {
                    (1. - u * u).powi(2)
                } else {
                    0.
                }
            })
            .collect(),
        _ => {
            let floor = 1e-6 * s;
            adj.iter().map(|a| 1. / a.abs().max(floor)).collect()
        }
    })
}

fn weighted_residuals(f: &[f64], d: &Data) -> Vec<f64> {
    f.iter()
        .zip(&d.y)
        .zip(&d.w)
        .map(|((f, y), w)| w.sqrt() * (y - f))
        .collect()
}

fn solve_linear(
    e: &mut Engine,
    ft: &FitType,
    d: &Data,
    problem: &[f64],
    opts: &Options,
    out: &mut Execution,
) -> Result<Solution> {
    let p = ft.coeffs.len();
    let basis = model::linear_basis(e, ft, problem, &d.refs(), out)?;
    let (lower, upper) = bounds(opts, p)?;
    let bounded = lower.iter().any(|v| v.is_finite()) || upper.iter().any(|v| v.is_finite());
    let m = d.y.len();
    let solve = |e: &mut Engine,
                 w: &[f64],
                 start: Option<&[f64]>,
                 out: &mut Execution|
     -> Result<(Vec<f64>, usize)> {
        let sw: Vec<f64> = w.iter().map(|v| v.sqrt()).collect();
        let mut a = basis.clone();
        for (i, s) in sw.iter().enumerate() {
            a.row_mut(i).scale_mut(*s);
        }
        let yw: Vec<f64> = d.y.iter().zip(&sw).map(|(y, s)| y * s).collect();
        let (mut b, rank) = solve::lstsq(&a, &yw);
        if bounded {
            if let Some(s) = start {
                b = s.to_vec();
            }
            let mut prob = Nls {
                e,
                out,
                ft,
                problem,
                xs: d.refs(),
                y: &d.y,
                sw,
                lower: lower.clone(),
                upper: upper.clone(),
                diff: (opts.diff_min, opts.diff_max),
            };
            let settings = Settings {
                lower: lower.clone(),
                upper: upper.clone(),
                max_iter: 1000,
                max_fev: 100_000,
                tol_fun: 1e-12,
                tol_x: 1e-12,
                iter_display: false,
            };
            let o = solve::levenberg_marquardt(&mut prob, &b, &settings)?;
            return Ok((o.x, p));
        }
        Ok((b, rank))
    };
    let (mut coef, rank) = solve(e, &d.w, None, out)?;
    if rank < p {
        warn(
            e,
            "curvefit:fit:rankDeficient",
            &format!(
                "The linear system is rank deficient (rank {rank} of {p}); dependent coefficients are set to 0. Remove repeated points or use 'Normalize','on'."
            ),
            out,
        )?;
    }
    let mut final_w = d.w.clone();
    let mut rounds = 0;
    if opts.robust != Robust::Off {
        let mut jw = basis.clone();
        for (i, w) in d.w.iter().enumerate() {
            jw.row_mut(i).scale_mut(w.sqrt());
        }
        let h = solve::leverages(&jw);
        for _ in 0..ROBUST_ROUNDS {
            e.check_cancel()?;
            rounds += 1;
            let f: Vec<f64> = (0..m)
                .map(|i| (0..p).map(|k| basis[(i, k)] * coef[k]).sum())
                .collect();
            let r = weighted_residuals(&f, d);
            let Some(rw) = robust_weights(opts.robust, &r, &h) else {
                break;
            };
            let w: Vec<f64> = d.w.iter().zip(&rw).map(|(a, b)| a * b).collect();
            let (next, _) = solve(e, &w, Some(&coef), out)?;
            final_w = w;
            let done = next
                .iter()
                .zip(&coef)
                .all(|(a, b)| (a - b).abs() <= 1e-13 * (1. + b.abs()));
            coef = next;
            if done {
                break;
            }
        }
    }
    let f: Vec<f64> = (0..m)
        .map(|i| (0..p).map(|k| basis[(i, k)] * coef[k]).sum())
        .collect();
    let r = weighted_residuals(&f, d);
    let firstorder = (0..p)
        .map(|k| {
            2. * (0..m)
                .map(|i| basis[(i, k)] * d.w[i].sqrt() * r[i])
                .sum::<f64>()
                .abs()
        })
        .fold(0., f64::max);
    Ok(Solution {
        coef,
        final_w,
        iterations: rounds.max(1),
        func_count: 1 + rounds,
        exitflag: 1,
        firstorder,
        algorithm: "QR factorization and solve",
        message: "Solved by QR factorization of the linear least-squares system.".into(),
    })
}

#[allow(clippy::too_many_arguments)]
fn solve_nonlinear(
    e: &mut Engine,
    ft: &FitType,
    d: &Data,
    problem: &[f64],
    opts: &Options,
    out: &mut Execution,
) -> Result<Solution> {
    let p = ft.coeffs.len();
    let (mut lower, mut upper) = bounds(opts, p)?;
    if opts.levenberg
        && (lower.iter().any(|v| v.is_finite()) || upper.iter().any(|v| v.is_finite()))
    {
        let default = match &ft.kind {
            Kind::Library(lib) => lib.default_lower(),
            _ => Vec::new(),
        };
        let user = !(opts.lower == default || opts.lower.is_empty()) || !opts.upper.is_empty();
        if user {
            warn(
                e,
                "curvefit:fit:boundsIgnored",
                "The Levenberg-Marquardt algorithm does not use bounds; Lower and Upper are ignored. Use 'Algorithm','Trust-Region' for a bounded fit.",
                out,
            )?;
        }
        lower = vec![f64::NEG_INFINITY; p];
        upper = vec![f64::INFINITY; p];
    }
    let start = if !opts.start.is_empty() {
        if opts.start.len() != p {
            return Err(format!(
                "fit: StartPoint must have one value per coefficient ({p}), not {}.",
                opts.start.len()
            ));
        }
        opts.start.clone()
    } else if let Kind::Library(lib) = &ft.kind {
        start::start_point(*lib, &d.xs[0], &d.y)
    } else {
        if opts.display != Display::Off {
            warn(
                e,
                "curvefit:fit:noStartPoint",
                "Start point not provided; using a fixed pseudo-random start point. Give StartPoint for a reliable fit.",
                out,
            )?;
        }
        start::seeded_start(&lower, &upper)
    };
    let settings = Settings {
        lower: lower.clone(),
        upper: upper.clone(),
        max_iter: opts.max_iter.min(1e7) as usize,
        max_fev: opts.max_fev.min(1e8) as usize,
        tol_fun: opts.tol_fun,
        tol_x: opts.tol_x,
        iter_display: opts.display == Display::Iter,
    };
    let run = |e: &mut Engine, w: &[f64], b0: &[f64], out: &mut Execution| {
        let mut prob = Nls {
            e,
            out,
            ft,
            problem,
            xs: d.refs(),
            y: &d.y,
            sw: w.iter().map(|v| v.sqrt()).collect(),
            lower: lower.clone(),
            upper: upper.clone(),
            diff: (opts.diff_min, opts.diff_max),
        };
        solve::levenberg_marquardt(&mut prob, b0, &settings)
    };
    let mut o = run(e, &d.w, &start, out)?;
    let (mut iterations, mut func_count) = (o.iterations, o.func_count);
    let mut final_w = d.w.clone();
    if opts.robust != Robust::Off {
        for _ in 0..ROBUST_ROUNDS {
            e.check_cancel()?;
            let f = model::values(e, ft, &o.x, problem, None, &d.refs(), out)?;
            let r = weighted_residuals(&f, d);
            let (mut j, _) = model::coefficient_jacobian(
                e,
                ft,
                &o.x,
                problem,
                &d.refs(),
                Some((&lower, &upper)),
                (opts.diff_min, opts.diff_max),
                out,
            )?;
            for (i, w) in d.w.iter().enumerate() {
                j.row_mut(i).scale_mut(w.sqrt());
            }
            let h = solve::leverages(&j);
            let Some(rw) = robust_weights(opts.robust, &r, &h) else {
                break;
            };
            let w: Vec<f64> = d.w.iter().zip(&rw).map(|(a, b)| a * b).collect();
            let next = run(e, &w, &o.x, out)?;
            iterations += next.iterations;
            func_count += next.func_count;
            final_w = w;
            let done = next
                .x
                .iter()
                .zip(&o.x)
                .all(|(a, b)| (a - b).abs() <= 1e-10 * (1. + b.abs()));
            o = next;
            if done {
                break;
            }
        }
    }
    if o.exitflag <= 0 && opts.display != Display::Off {
        warn(
            e,
            "curvefit:fit:noConvergence",
            &format!(
                "The fit did not converge. {} Increase MaxIter or MaxFunEvals in the fit options, or give a better StartPoint.",
                o.message
            ),
            out,
        )?;
    }
    if opts.display == Display::Final {
        e.emit(out, format!("{}\n", o.message))?;
    }
    Ok(Solution {
        coef: o.x,
        final_w,
        iterations,
        func_count,
        exitflag: o.exitflag,
        firstorder: o.firstorder,
        algorithm: if opts.levenberg {
            "levenberg-marquardt"
        } else {
            "trust-region-reflective"
        },
        message: o.message,
    })
}

/// Sorted distinct sites with averaged values.
fn knots(x: &[f64], y: &[f64]) -> Result<Knots> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let (mut kx, mut ky, mut count): (Vec<f64>, Vec<f64>, Vec<f64>) = (vec![], vec![], vec![]);
    for i in idx {
        if kx.last() == Some(&x[i]) {
            *ky.last_mut().unwrap() += y[i];
            *count.last_mut().unwrap() += 1.;
        } else {
            kx.push(x[i]);
            ky.push(y[i]);
            count.push(1.);
        }
    }
    for (v, c) in ky.iter_mut().zip(&count) {
        *v /= c;
    }
    if kx.len() < 2 {
        return Err("fit: an interpolant needs at least two distinct x values.".into());
    }
    Ok(Knots {
        x: kx,
        y: ky,
        smooth: None,
    })
}

fn column(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(v.len(), 1, v)?))
}
fn number(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}

/// [fitobject, gof, output] = fit(x, y, model, ...)
pub(super) fn fit(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.len() < 3 {
        return Err(
            "fit: expected fit(x, y, fitType) with optional fit options and name-value pairs"
                .into(),
        );
    }
    let ft = model_arg(e, &args[2], out)?;
    let mut opts = Options::for_model(&ft);
    let mut rest = &args[3..];
    if let Some(Value::Container(Array::Struct(s))) = rest.first() {
        opts.merge_struct(e, s)?;
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("fit: name-value arguments must come in pairs".into());
    }
    let mut problem = Vec::new();
    for pair in rest.chunks(2) {
        let name = text_of(&pair[0]).ok_or("fit: option names must be text")?;
        if name.eq_ignore_ascii_case("problem") {
            problem = problem_values(e, &pair[1])?;
        } else {
            opts.set(e, &name, &pair[1])?;
        }
    }
    if problem.len() != ft.problems.len() {
        return Err(format!(
            "fit: the model has {} problem parameter(s) ({}); give one value each with 'problem'.",
            ft.problems.len(),
            ft.problems.join(", ")
        ));
    }
    let d = prepare(e, &ft, &args[0], &args[1], &opts)?;
    let m = d.y.len();
    let p = ft.coeffs.len();
    let (sol, knots, numparam) = match opts.method {
        Method::Interp(m) => {
            let k = if m == Interp::Smooth {
                super::smooth::knots(&d.xs[0], &d.y, &d.w, opts.smoothing)?
            } else {
                knots(&d.xs[0], &d.y)?
            };
            let n = k.x.len();
            let sol = Solution {
                coef: vec![f64::NAN],
                final_w: d.w.clone(),
                iterations: 0,
                func_count: 0,
                exitflag: 1,
                firstorder: 0.,
                algorithm: if k.smooth.is_some() {
                    "smoothing spline"
                } else {
                    "interpolation"
                },
                message: if k.smooth.is_some() {
                    "The smoothing spline minimizes p*sse + (1-p)*roughness.".into()
                } else {
                    "The interpolant passes through the data.".into()
                },
            };
            (sol, Some(k), n)
        }
        _ => {
            if m < p {
                return Err(format!(
                    "fit: insufficient data: the model has {p} coefficients but only {m} data points are used."
                ));
            }
            let sol = if opts.method == Method::Linear {
                solve_linear(e, &ft, &d, &problem, &opts, out)?
            } else {
                solve_nonlinear(e, &ft, &d, &problem, &opts, out)?
            };
            (sol, None, p)
        }
    };
    let interp = knots.is_some();
    let f = model::values(e, &ft, &sol.coef, &problem, knots.as_ref(), &d.refs(), out)?;
    let resid: Vec<f64> = d.y.iter().zip(&f).map(|(y, f)| y - f).collect();
    let sse: f64 = resid.iter().zip(&d.w).map(|(r, w)| w * r * r).sum();
    let wsum: f64 = d.w.iter().sum();
    let ybar = d.y.iter().zip(&d.w).map(|(y, w)| w * y).sum::<f64>() / wsum;
    let sst: f64 =
        d.y.iter()
            .zip(&d.w)
            .map(|(y, w)| w * (y - ybar).powi(2))
            .sum();
    let dfe = if interp { 0. } else { (m - p) as f64 };
    if !interp && dfe == 0. {
        warn(
            e,
            "curvefit:fit:noDegreesOfFreedom",
            "The fit has as many coefficients as data points (no error degrees of freedom); confidence bounds and adjusted statistics are NaN.",
            out,
        )?;
    }
    let rsquare = if sst > 0. { 1. - sse / sst } else { f64::NAN };
    let adj = if dfe > 0. && sst > 0. {
        1. - sse / sst * (m as f64 - 1.) / dfe
    } else {
        f64::NAN
    };
    let rmse = if dfe > 0. {
        (sse / dfe).sqrt()
    } else {
        f64::NAN
    };
    // Coefficient covariance from the weighted Jacobian of the last round.
    let (jac, _) = if interp {
        (DMatrix::zeros(m, 0), 0)
    } else {
        model::coefficient_jacobian(
            e,
            &ft,
            &sol.coef,
            &problem,
            &d.refs(),
            None,
            (opts.diff_min, opts.diff_max),
            out,
        )?
    };
    let (cov, mse) = if interp || dfe <= 0. {
        (Vec::new(), f64::NAN)
    } else {
        let mut jw = jac.clone();
        for (i, w) in sol.final_w.iter().enumerate() {
            jw.row_mut(i).scale_mut(w.sqrt());
        }
        let wsse: f64 = resid.iter().zip(&sol.final_w).map(|(r, w)| w * r * r).sum();
        // A coefficient fixed by Lower == Upper has no variance: NaN bounds.
        let np = jw.ncols();
        let (lower, upper) = bounds(&opts, np)?;
        let free: Vec<usize> = (0..np).filter(|&k| lower[k] != upper[k]).collect();
        let cov = if free.len() == np {
            solve::covariance(&jw)
        } else {
            let part = solve::covariance(&jw.select_columns(free.iter()));
            let mut cov = vec![f64::NAN; np * np];
            for (a, &i) in free.iter().enumerate() {
                for (b, &j) in free.iter().enumerate() {
                    cov[i * np + j] = part[a * free.len() + b];
                }
            }
            cov
        };
        (cov, wsse / dfe)
    };
    let smoothing = knots.as_ref().and_then(|k| k.smooth.as_ref()).map(|s| s.p);
    let fitted = Fitted {
        ft: ft.clone(),
        coef: sol.coef.clone(),
        problem: problem.clone(),
        cov,
        mse,
        dfe,
        norm: d.norm.clone(),
        knots,
        range: d.range.clone(),
    };
    let mut values = vec![super::new_object(e, super::FitObj::Fit(fitted))?];
    if nargout >= 2 {
        let fields: Vec<(&str, Value)> = vec![
            ("sse", number(sse)),
            ("rsquare", number(rsquare)),
            ("dfe", number(dfe)),
            ("adjrsquare", number(adj)),
            ("rmse", number(rmse)),
        ];
        values.push(super::make_struct(e, fields)?);
    }
    if nargout >= 3 {
        let jac_value = Value::Matrix(Matrix::from_column_vec(
            jac.nrows(),
            jac.ncols(),
            jac.as_slice().to_vec(),
        )?);
        let mut fields: Vec<(&str, Value)> = vec![
            ("numobs", number(m as f64)),
            ("numparam", number(numparam as f64)),
            ("residuals", column(resid)?),
            ("Jacobian", jac_value),
            ("exitflag", number(sol.exitflag as f64)),
            ("iterations", number(sol.iterations as f64)),
            ("funcCount", number(sol.func_count as f64)),
            ("firstorderopt", number(sol.firstorder)),
            ("algorithm", Value::Text(sol.algorithm.into())),
            ("message", Value::Text(sol.message)),
        ];
        if let Some(p) = smoothing {
            fields.push(("p", number(p)));
        }
        values.push(super::make_struct(e, fields)?);
    }
    if nargout > 3 {
        return Err("fit: at most three outputs ([fitobject, gof, output])".into());
    }
    Ok(values)
}
