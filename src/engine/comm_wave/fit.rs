//! BER closed forms from the public berfit Algorithms table. The engine's
//! script-only fminsearch cannot accept a Rust closure/options; this bounded
//! Nelder-Mead follows Nelder & Mead (1965), with reflection/expansion/
//! contraction/shrink coefficients 1/2/0.5/0.5. Objective is squared log BER.
use super::*;
use crate::{CharacterRow, StructArray};
use nalgebra::{DMatrix, DVector};
use std::sync::Arc;
const TYPES: [&str; 4] = ["exp", "exp+const", "polyRatio", "doubleExp+const"];
#[derive(Clone)]
struct Options {
    evals: usize,
    iters: usize,
    tolx: f64,
    tolf: f64,
    display: String,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            evals: 10000,
            iters: 10000,
            tolx: 1e-4,
            tolf: 1e-4,
            display: "off".into(),
        }
    }
}
fn options(e: &Engine, v: Option<&Value>) -> Result<Options> {
    let mut o = Options::default();
    let Some(v) = v.filter(|v| !empty(v)) else {
        return Ok(o);
    };
    let Value::Container(Array::Struct(s)) = v else {
        return Err("berfit: options must be a scalar structure or []".into());
    };
    if s.len() != 1 {
        return Err("berfit: options must be scalar".into());
    }
    for (i, key) in s.field_names().iter().enumerate() {
        let v = Value::from(s.get(i, 0)?.as_ref().clone());
        if empty(&v) {
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "maxfunevals" | "maxfunctionevaluations" => {
                o.evals = integer(e, &v, 1, 1_000_000, "berfit MaxFunEvals")?
            }
            "maxiter" | "maxiterations" => {
                o.iters = integer(e, &v, 1, 1_000_000, "berfit MaxIter")?
            }
            "tolx" | "steptolerance" => o.tolx = positive(e, &v, "berfit TolX")?,
            "tolfun" | "functiontolerance" => o.tolf = positive(e, &v, "berfit TolFun")?,
            "display" => {
                o.display = choice(
                    e,
                    Some(&v),
                    "off",
                    &["off", "iter", "final", "notify"],
                    "berfit Display",
                )?
            }
            _ => return Err(format!("berfit: unsupported optimization option '{key}'")),
        }
    }
    Ok(o)
}
fn logadd(a: f64, b: f64) -> f64 {
    let m = a.max(b);
    if m == f64::NEG_INFINITY {
        m
    } else {
        m + ((a - m).exp() + (b - m).exp()).ln()
    }
}
fn term(p: &[f64], x: f64, xmin: f64) -> f64 {
    let base = x - xmin + p[1].exp();
    if base <= 0. {
        return f64::NAN;
    }
    p[0] - (p[2].exp() * base.ln() - p[3]).exp()
}
fn log_model(kind: usize, p: &[f64], x: f64, xmin: f64) -> f64 {
    match kind {
        0 => term(p, x, xmin),
        1 => logadd(term(p, x, xmin), p[4]),
        2 => {
            let numerator = (p[0] * x + p[1]) * x + p[2];
            let denominator = ((x + p[3]) * x + p[4]) * x + p[5];
            let v = numerator / denominator;
            if v > 0. { v.ln() } else { f64::NAN }
        }
        _ => logadd(logadd(term(p, x, xmin), term(&p[4..], x, xmin)), p[8]),
    }
}
struct Solution {
    p: Vec<f64>,
    error: f64,
    evals: usize,
    iters: usize,
    converged: bool,
}
fn nelder(
    e: &mut Engine,
    cost: usize,
    initial: &[f64],
    o: &Options,
    mut objective: impl FnMut(&[f64]) -> f64,
    out: &mut Execution,
) -> Result<Solution> {
    let n = initial.len();
    let evals = std::cell::Cell::new(0);
    let mut eval = |e: &mut Engine, p: &[f64]| -> Result<f64> {
        e.check_cancel()?;
        if evals.get() >= o.evals {
            return Ok(f64::INFINITY);
        }
        evals.set(evals.get() + 1);
        work(e, cost)?;
        let f = objective(p);
        Ok(if f.is_finite() { f } else { f64::INFINITY })
    };
    let mut simplex = vec![initial.to_vec(); n + 1];
    for (j, p) in simplex.iter_mut().skip(1).enumerate() {
        p[j] += 0.1_f64.max(p[j].abs() * 0.05);
    }
    let mut points = simplex
        .into_iter()
        .map(|p| eval(e, &p).map(|f| (p, f)))
        .collect::<Result<Vec<_>>>()?;
    let mut converged = false;
    let mut iters = 0;
    for iter in 0..o.iters {
        e.check_cancel()?;
        points.sort_by(|a, b| a.1.total_cmp(&b.1));
        iters = iter;
        let spread = points[1..]
            .iter()
            .flat_map(|(p, _)| p.iter().zip(&points[0].0).map(|(a, b)| (a - b).abs()))
            .fold(0., f64::max);
        if points[0].1.is_finite() && spread <= o.tolx && points[n].1 - points[0].1 <= o.tolf {
            converged = true;
            break;
        }
        if evals.get() >= o.evals {
            break;
        }
        let centroid = (0..n)
            .map(|j| points[..n].iter().map(|(p, _)| p[j]).sum::<f64>() / n as f64)
            .collect::<Vec<_>>();
        let reflect = (0..n)
            .map(|j| 2. * centroid[j] - points[n].0[j])
            .collect::<Vec<_>>();
        let fr = eval(e, &reflect)?;
        if fr < points[0].1 {
            let expand = (0..n)
                .map(|j| centroid[j] + 2. * (reflect[j] - centroid[j]))
                .collect::<Vec<_>>();
            let fe = eval(e, &expand)?;
            points[n] = if fe < fr { (expand, fe) } else { (reflect, fr) };
        } else if fr < points[n - 1].1 {
            points[n] = (reflect, fr);
        } else {
            let outside = fr < points[n].1;
            let contract = (0..n)
                .map(|j| {
                    centroid[j]
                        + 0.5 * (if outside { reflect[j] } else { points[n].0[j] } - centroid[j])
                })
                .collect::<Vec<_>>();
            let fc = eval(e, &contract)?;
            if fc < if outside { fr } else { points[n].1 } {
                points[n] = (contract, fc);
            } else {
                let best = points[0].0.clone();
                for (p, f) in &mut points[1..] {
                    for j in 0..n {
                        p[j] = (p[j] + best[j]) / 2.;
                    }
                    *f = eval(e, p)?;
                }
            }
        }
        if o.display == "iter" {
            e.emit(
                out,
                format!(
                    "berfit iteration {}: log BER sumSqErr = {:.8e}",
                    iter + 1,
                    points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min)
                ),
            )?;
        }
        iters = iter + 1;
    }
    points.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (p, error) = points.remove(0);
    Ok(Solution {
        p,
        error,
        evals: evals.get(),
        iters,
        converged,
    })
}
fn initial(kind: usize, x: &[f64], ly: &[f64], scale: f64) -> Vec<f64> {
    if kind == 2 {
        let rows = x.len();
        let a = DMatrix::from_fn(rows, 6, |i, j| {
            let y = ly[i].exp();
            let z = x[i];
            match j {
                0 => z * z / y,
                1 => z / y,
                2 => 1. / y,
                3 => -z * z,
                4 => -z,
                _ => -1.,
            }
        });
        let b = DVector::from_iterator(rows, x.iter().map(|v| v.powi(3)));
        if let Ok(p) = a.svd(true, true).solve(&b, 1e-13) {
            return p.iter().copied().collect();
        }
        return vec![0., 0., 0.1, 0., 0., 1.];
    }
    let meanx = x.iter().sum::<f64>() / x.len() as f64;
    let meany = ly.iter().sum::<f64>() / ly.len() as f64;
    let slope = (x
        .iter()
        .zip(ly)
        .map(|(x, y)| (x - meanx) * (y - meany))
        .sum::<f64>()
        / x.iter().map(|v| (v - meanx).powi(2)).sum::<f64>())
    .min(-1e-8);
    let p = vec![
        meany - slope * meanx,
        x[0].max(1e-8).ln(),
        0.,
        (-1. / slope).ln(),
    ];
    if kind == 0 {
        p
    } else if kind == 1 {
        let mut p = p;
        p.push(ly[ly.len() - 1] - 10.);
        p
    } else {
        let mut q = p.clone();
        q[0] -= 3.;
        q[2] = 1.2_f64.ln();
        q[3] += (scale / scale.max(1.)).ln();
        let mut p = p;
        p.extend(q);
        p.push(ly[ly.len() - 1] - 10.);
        p
    }
}
fn coeffs(kind: usize, p: &[f64], xmin: f64, scale: f64) -> Vec<f64> {
    let convert = |p: &[f64]| {
        vec![
            p[0].exp(),
            (xmin - p[1].exp()) * scale,
            p[2].exp(),
            p[3].exp() * scale.powf(p[2].exp()),
        ]
    };
    match kind {
        0 => convert(p),
        1 => {
            let mut v = convert(p);
            v.push(p[4].exp());
            v
        }
        2 => vec![
            p[0] * scale,
            p[1] * scale.powi(2),
            p[2] * scale.powi(3),
            p[3] * scale,
            p[4] * scale.powi(2),
            p[5] * scale.powi(3),
        ],
        _ => {
            let mut v = convert(p);
            v.extend(convert(&p[4..]));
            v.push(p[8].exp());
            v
        }
    }
}
fn properties(
    e: &Engine,
    kind: usize,
    s: &Solution,
    xmin: f64,
    scale: f64,
    valid: bool,
) -> Result<Value> {
    let text =
        |s: &str| -> Result<Array> { Ok(Array::Character(CharacterRow::from_text(s, &e.cancel)?)) };
    let fields = vec![
        "fitType",
        "coeffs",
        "sumSqErr",
        "exitState",
        "funcCount",
        "iterations",
    ];
    let coeff = if valid {
        coeffs(kind, &s.p, xmin, scale)
    } else {
        vec![]
    };
    let slots = vec![
        text(TYPES[kind])?,
        Array::Numeric(Matrix::from_column_vec(coeff.len(), 1, coeff)?),
        Array::Numeric(Matrix::scalar(s.error)),
        text(if !valid {
            "No desirable fit was found."
        } else if s.converged {
            "The curve fit converged to a solution."
        } else {
            "The maximum number of function evaluations was exceeded."
        })?,
        Array::Numeric(Matrix::scalar(s.evals as f64)),
        Array::Numeric(Matrix::scalar(s.iters as f64)),
    ];
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        fields.into_iter().map(str::to_string).collect(),
        slots.into_iter().map(Arc::new).collect(),
        &e.cancel,
    )?)))
}
pub(super) fn call(
    e: &mut Engine,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    arity(a, 2, 5, "berfit")?;
    let x = vector(e, &a[0], "berfit empEbNo")?;
    let y = vector(e, &a[1], "berfit empBER")?;
    if x.len() < 4 || x.len() != y.len() || x.len() > 65536 {
        return Err("berfit: empirical vectors must have equal length, between 4 and 65536".into());
    }
    if x.data().windows(2).any(|w| w[1] <= w[0]) || y.data().iter().any(|v| *v <= 0. || *v > 1.) {
        return Err("berfit: EbNo must be strictly ascending and BER must be in (0,1]".into());
    }
    let fx = match a.get(2).filter(|v| !empty(v)) {
        Some(v) => vector(e, v, "berfit fitEbNo")?,
        None => x.clone(),
    };
    if fx.len() < x.len() || fx.len() > MAX_SAMPLES || fx.data().windows(2).any(|w| w[1] <= w[0]) {
        return Err(
            "berfit: fitEbNo must be ascending with at least as many points as empEbNo".into(),
        );
    }
    let o = options(e, a.get(3))?;
    let typ = match a.get(4) {
        Some(v) => text(e, v, "berfit fittype")?,
        None => "best".into(),
    };
    if typ.eq_ignore_ascii_case("all") && nout > 0 {
        return Err("berfit: fittype 'all' is only supported without output arguments".into());
    }
    let kinds = if typ == "best" || typ.eq_ignore_ascii_case("all") {
        (0..4).collect::<Vec<_>>()
    } else {
        vec![
            TYPES
                .iter()
                .position(|t| t.eq_ignore_ascii_case(&typ))
                .ok_or("berfit: unknown fittype")?,
        ]
    };
    reserve(
        e,
        x.len()
            .saturating_mul(24)
            .saturating_add(fx.len().saturating_mul(16)),
    )?;
    let linear = x
        .data()
        .iter()
        .map(|v| 10_f64.powf(v / 10.))
        .collect::<Vec<_>>();
    let scale = *linear.last().unwrap();
    if !scale.is_finite() || linear[0] <= 0. {
        return Err("berfit: EbNo exceeds the finite linear range".into());
    }
    let normalized = linear.iter().map(|v| v / scale).collect::<Vec<_>>();
    let xmin = normalized[0];
    let ly = y.data().iter().map(|v| v.ln()).collect::<Vec<_>>();
    let fitx = fx
        .data()
        .iter()
        .map(|v| 10_f64.powf(v / 10.) / scale)
        .collect::<Vec<_>>();
    if fitx.iter().any(|x| !x.is_finite() || *x <= 0.) {
        return Err("berfit: fitEbNo exceeds the finite linear range".into());
    }
    let mut fits = Vec::new();
    for kind in kinds {
        let start = initial(kind, &normalized, &ly, scale);
        let solution = nelder(
            e,
            x.len(),
            &start,
            &o,
            |p| {
                normalized
                    .iter()
                    .zip(&ly)
                    .map(|(x, y)| {
                        let f = log_model(kind, p, *x, xmin);
                        if !f.is_finite() || f > 0. {
                            f64::INFINITY
                        } else {
                            (f - y).powi(2)
                        }
                    })
                    .sum()
            },
            out,
        )?;
        let fit = fitx
            .iter()
            .map(|x| log_model(kind, &solution.p, *x, xmin).exp())
            .collect::<Vec<_>>();
        let valid = solution.error.is_finite()
            && fit.iter().all(|y| y.is_finite() && *y >= 0. && *y <= 1.)
            && fit.windows(2).all(|w| w[1] <= w[0]);
        if o.display == "final" || (o.display == "notify" && (!valid || !solution.converged)) {
            e.emit(
                out,
                format!(
                    "berfit {}: sumSqErr = {:.8e}; {} evaluations",
                    TYPES[kind], solution.error, solution.evals
                ),
            )?;
        }
        fits.push((kind, solution, fit, valid));
    }
    fits.sort_by(|a, b| b.3.cmp(&a.3).then(a.1.error.total_cmp(&b.1.error)));
    if nout == 0 {
        let shown = fits
            .iter()
            .filter(|f| f.3)
            .take(if typ.eq_ignore_ascii_case("all") {
                4
            } else {
                1
            })
            .map(|f| (format!("{} Fit", TYPES[f.0]), f.2.clone()))
            .collect::<Vec<_>>();
        plots::ber(e, x.data(), y.data(), fx.data(), &shown, out)?;
    }
    let (kind, solution, fit, valid) = fits.remove(0);
    let mut result = vec![if valid {
        rv(fx.rows(), fx.cols(), fit)?
    } else {
        rv(0, 0, vec![])?
    }];
    if nout > 1 {
        result.push(properties(e, kind, &solution, xmin, scale, valid)?);
    }
    Ok(result)
}
