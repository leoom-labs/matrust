//! probplot, normplot, wblplot, qqplot, ecdf and cdfplot, drawn with the
//! ordinary plot/stairs/text builtins.
//!
//! Plotting positions are (i - 0.5)/n for uncensored data, as the public
//! probplot page states; with censoring or frequencies they are the
//! midpoints of the Kaplan-Meier steps. The probability axis is the
//! distribution's standardized quantile scale; probability labels are text
//! objects next to dotted grid lines at the transformed positions. The
//! reference line passes through the first and third quartiles (the minimum
//! and median for the half normal). ecdf is the Kaplan-Meier estimate with
//! Greenwood's variance (M. Greenwood, 1926).
use super::super::dist_objects::Distribution;
use super::super::stats_dist::families::Family;
use super::super::{Engine, Execution, Value};
use super::sample::{self, Sample};
use crate::{Array, GraphicsKind, Matrix, Result};

fn text(s: &str) -> Value {
    Value::Text(s.into())
}
fn column(v: Vec<f64>) -> Value {
    let n = v.len();
    Value::Matrix(Matrix::from_column_vec(n, 1, v).expect("column"))
}
fn row(v: Vec<f64>) -> Value {
    let n = v.len();
    Value::Matrix(Matrix::from_column_vec(1, n, v).expect("row"))
}
fn number(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}
fn norm_inv(p: f64) -> f64 {
    Family::Norm.inv(p, &[0., 1.])
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let max = match name {
        "ecdf" => 4,
        "cdfplot" => 2,
        _ => 1,
    };
    if nargout > max {
        return Err(format!("{name}: too many output arguments"));
    }
    let mut args = args.to_vec();
    let mut axes = None;
    if let Some(Value::Container(Array::Graphics(h))) = args.first()
        && h.len() == 1
        && h.scalar_handle()?.kind() == GraphicsKind::Axes
    {
        axes = Some(args.remove(0));
    }
    let old = e.figure.clone();
    let result = (|| {
        if let Some(ax) = &axes {
            e.call("axes", vec![ax.clone()], out)?;
        }
        match name {
            "ecdf" => ecdf(e, &args, nargout, out),
            "cdfplot" => cdfplot(e, &args, nargout, out),
            "qqplot" => qqplot(e, &args, nargout, out),
            "normplot" => probplot(e, Some(Scale::Normal), &args, nargout, out, axes.is_some()),
            "wblplot" => probplot(e, Some(Scale::Weibull), &args, nargout, out, axes.is_some()),
            _ => probplot(e, None, &args, nargout, out, axes.is_some()),
        }
    })();
    if result.is_err() {
        e.figure = old;
    }
    result
}

/// Runs `f` with hold on and restores the previous hold state.
fn held<T>(
    e: &mut Engine,
    out: &mut Execution,
    f: impl FnOnce(&mut Engine, &mut Execution) -> Result<T>,
) -> Result<T> {
    let was = e.figure.as_ref().is_some_and(|f| f.hold);
    e.call("hold", vec![text("on")], out)?;
    let result = f(e, out);
    if !was {
        e.call("hold", vec![text("off")], out)?;
    }
    result
}

/// Percentile of sorted data by the midpoint rule (as prctile).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let pos = n as f64 * p + 0.5;
    if pos <= 1. {
        return sorted[0];
    }
    if pos >= n as f64 {
        return sorted[n - 1];
    }
    let k = pos.floor() as usize;
    let t = pos - k as f64;
    sorted[k - 1] + t * (sorted[k] - sorted[k - 1])
}

// ---------- Kaplan-Meier ----------

/// Distinct failure times with (cdf before, cdf after, Greenwood sum,
/// Nelson-Aalen hazard and its variance).
struct Km {
    first: f64,
    times: Vec<f64>,
    survival: Vec<f64>,
    greenwood: Vec<f64>,
    hazard: Vec<f64>,
    hazard_var: Vec<f64>,
}

fn kaplan_meier(s: &Sample) -> Km {
    let mut obs: Vec<(f64, bool, f64)> = s.iter().collect();
    obs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut at_risk: f64 = obs.iter().map(|o| o.2).sum();
    let mut km = Km {
        first: obs[0].0,
        times: Vec::new(),
        survival: Vec::new(),
        greenwood: Vec::new(),
        hazard: Vec::new(),
        hazard_var: Vec::new(),
    };
    let (mut surv, mut gw, mut haz, mut hv) = (1., 0., 0., 0.);
    let mut i = 0;
    while i < obs.len() {
        let t = obs[i].0;
        let (mut d, mut c) = (0., 0.);
        while i < obs.len() && obs[i].0 == t {
            if obs[i].1 {
                c += obs[i].2;
            } else {
                d += obs[i].2;
            }
            i += 1;
        }
        if d > 0. {
            surv *= 1. - d / at_risk;
            gw += d / (at_risk * (at_risk - d));
            haz += d / at_risk;
            hv += d / (at_risk * at_risk);
            km.times.push(t);
            km.survival.push(surv);
            km.greenwood.push(gw);
            km.hazard.push(haz);
            km.hazard_var.push(hv);
        }
        at_risk -= d + c;
    }
    km
}

#[derive(Clone, Copy, PartialEq)]
enum EcdfFunction {
    Cdf,
    Survivor,
    Hazard,
}

fn ecdf_values(
    s: &Sample,
    function: EcdfFunction,
    alpha: f64,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let km = kaplan_meier(s);
    let z = norm_inv(1. - alpha / 2.);
    let mut x = vec![km.first];
    let start = if function == EcdfFunction::Survivor {
        1.
    } else {
        0.
    };
    let (mut f, mut lo, mut hi) = (vec![start], vec![f64::NAN], vec![f64::NAN]);
    for k in 0..km.times.len() {
        x.push(km.times[k]);
        let (value, se) = match function {
            EcdfFunction::Cdf | EcdfFunction::Survivor => {
                let sv = km.survival[k];
                let se = (sv * sv * km.greenwood[k]).sqrt();
                (
                    if function == EcdfFunction::Cdf {
                        1. - sv
                    } else {
                        sv
                    },
                    se,
                )
            }
            EcdfFunction::Hazard => (km.hazard[k], km.hazard_var[k].sqrt()),
        };
        f.push(value);
        let top = if function == EcdfFunction::Hazard {
            f64::INFINITY
        } else {
            1.
        };
        lo.push((value - z * se).clamp(0., top));
        hi.push((value + z * se).clamp(0., top));
    }
    // NaN variance (all remaining units failed) leaves NaN bounds.
    for k in 1..f.len() {
        if hi[k].is_nan() || lo[k].is_nan() {
            lo[k] = f64::NAN;
            hi[k] = f64::NAN;
        }
    }
    (f, x, lo, hi)
}

fn ecdf(e: &mut Engine, args: &[Value], nargout: usize, out: &mut Execution) -> Result<Vec<Value>> {
    let Some(data) = args.first() else {
        return Err("ecdf: expected a data vector".into());
    };
    let rest = &args[1..];
    if !rest.len().is_multiple_of(2) {
        return Err("ecdf: options must be name/value pairs".into());
    }
    let (mut cens, mut freq, mut alpha, mut function, mut bounds) =
        (None, None, 0.05, EcdfFunction::Cdf, false);
    for pair in rest.as_chunks::<2>().0 {
        let Value::Text(key) = &pair[0] else {
            return Err("ecdf: option names must be text".into());
        };
        let word = |v: &Value| match v {
            Value::Text(t) => Ok(t.to_ascii_lowercase()),
            _ => Err(format!("ecdf: '{key}' must be text")),
        };
        match key.to_ascii_lowercase().as_str() {
            "censoring" => cens = Some(pair[1].clone()),
            "frequency" => freq = Some(pair[1].clone()),
            "alpha" => {
                let a = e.real_arg("ecdf", &pair[1])?.as_scalar()?;
                if !(a > 0. && a < 1.) {
                    return Err("ecdf: alpha must be between 0 and 1".into());
                }
                alpha = a;
            }
            "function" => {
                function = match word(&pair[1])?.as_str() {
                    "cdf" => EcdfFunction::Cdf,
                    "survivor" => EcdfFunction::Survivor,
                    "cumulative hazard" => EcdfFunction::Hazard,
                    other => {
                        return Err(format!(
                            "ecdf: 'Function' must be 'cdf', 'survivor' or 'cumulative hazard' (got '{other}')"
                        ));
                    }
                }
            }
            "bounds" => {
                bounds = match word(&pair[1])?.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("ecdf: 'Bounds' must be 'on' or 'off'".into()),
                }
            }
            _ => return Err(format!("ecdf: unsupported option '{key}'")),
        }
    }
    let s = sample::sample(e, "ecdf", data, cens.as_ref(), freq.as_ref())?;
    let (f, x, lo, hi) = ecdf_values(&s, function, alpha);
    if nargout > 0 {
        return Ok(vec![column(f), column(x), column(lo), column(hi)]
            .into_iter()
            .take(nargout)
            .collect());
    }
    e.call("stairs", vec![column(x.clone()), column(f)], out)?;
    if bounds {
        held(e, out, |e, out| {
            e.call(
                "stairs",
                vec![column(x.clone()), column(lo), text("r:")],
                out,
            )?;
            e.call("stairs", vec![column(x), column(hi), text("r:")], out)
        })?;
    }
    e.call("xlabel", vec![text("x")], out)?;
    let label = match function {
        EcdfFunction::Cdf => "F(x)",
        EcdfFunction::Survivor => "S(x)",
        EcdfFunction::Hazard => "H(x)",
    };
    e.call("ylabel", vec![text(label)], out)?;
    Ok(Vec::new())
}

fn cdfplot(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let [data] = args else {
        return Err("cdfplot: expected one data vector".into());
    };
    let s = sample::sample(e, "cdfplot", data, None, None)?;
    let (f, x, _, _) = ecdf_values(&s, EcdfFunction::Cdf, 0.05);
    let h = e.call("stairs", vec![column(x), column(f)], out)?;
    e.call("grid", vec![text("on")], out)?;
    e.call("title", vec![text("Empirical CDF")], out)?;
    e.call("xlabel", vec![text("x")], out)?;
    e.call("ylabel", vec![text("F(x)")], out)?;
    let mut outputs = vec![h];
    if nargout > 1 {
        let mut sorted = s.x.clone();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let median = if n % 2 == 1 {
            sorted[n / 2]
        } else {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.
        };
        let plain = Sample::plain(s.x.clone());
        let std = if n > 1 { plain.std() } else { 0. };
        let num = |v: f64| Array::Numeric(Matrix::scalar(v));
        outputs.push(super::super::optim_nl::onl_struct(
            e,
            vec![
                ("min", num(sorted[0])),
                ("max", num(sorted[n - 1])),
                ("mean", num(plain.mean())),
                ("median", num(median)),
                ("std", num(std)),
            ],
        )?);
    }
    Ok(outputs)
}

// ---------- probability plots ----------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Scale {
    Normal,
    Exponential,
    ExtremeValue,
    HalfNormal,
    Lognormal,
    Logistic,
    Loglogistic,
    Rayleigh,
    Weibull,
}

const SCALES: [Scale; 9] = [
    Scale::Normal,
    Scale::Exponential,
    Scale::ExtremeValue,
    Scale::HalfNormal,
    Scale::Lognormal,
    Scale::Logistic,
    Scale::Loglogistic,
    Scale::Rayleigh,
    Scale::Weibull,
];

impl Scale {
    fn parse(name: &str) -> Result<Self> {
        let key: String = name
            .to_ascii_lowercase()
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '_'))
            .collect();
        Ok(match key.as_str() {
            "normal" | "norm" => Self::Normal,
            "exponential" | "exp" => Self::Exponential,
            "extremevalue" | "ev" => Self::ExtremeValue,
            "halfnormal" => Self::HalfNormal,
            "lognormal" | "logn" => Self::Lognormal,
            "logistic" => Self::Logistic,
            "loglogistic" => Self::Loglogistic,
            "rayleigh" | "rayl" => Self::Rayleigh,
            "weibull" | "wbl" => Self::Weibull,
            _ => {
                return Err(format!(
                    "probplot: unknown distribution '{name}'; expected normal, exponential, extreme value, half normal, lognormal, logistic, loglogistic, rayleigh or weibull"
                ));
            }
        })
    }
    fn of_family(f: Family) -> Option<Self> {
        Some(match f {
            Family::Norm => Self::Normal,
            Family::Exp => Self::Exponential,
            Family::Ev => Self::ExtremeValue,
            Family::HalfNorm => Self::HalfNormal,
            Family::Logn => Self::Lognormal,
            Family::Logistic => Self::Logistic,
            Family::Loglogistic => Self::Loglogistic,
            Family::Rayl => Self::Rayleigh,
            Family::Wbl => Self::Weibull,
            _ => return None,
        })
    }
    fn title(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Exponential => "Exponential",
            Self::ExtremeValue => "Extreme Value",
            Self::HalfNormal => "Half Normal",
            Self::Lognormal => "Lognormal",
            Self::Logistic => "Logistic",
            Self::Loglogistic => "Loglogistic",
            Self::Rayleigh => "Rayleigh",
            Self::Weibull => "Weibull",
        }
    }
    fn log_x(self) -> bool {
        matches!(self, Self::Lognormal | Self::Loglogistic | Self::Weibull)
    }
    /// The standardized quantile at probability p (the y coordinate).
    fn y(self, p: f64) -> f64 {
        match self {
            Self::Normal | Self::Lognormal => norm_inv(p),
            Self::Exponential => -(-p).ln_1p(),
            Self::ExtremeValue | Self::Weibull => (-(-p).ln_1p()).ln(),
            Self::HalfNormal => norm_inv(0.5 + p / 2.),
            Self::Logistic | Self::Loglogistic => super::families::logit(p),
            Self::Rayleigh => (-2. * (-p).ln_1p()).sqrt(),
        }
    }
    fn title_text(self) -> String {
        format!("Probability plot for {} distribution", self.title())
    }
    /// The scale a previous probplot left in the axes title.
    fn from_title(title: &str) -> Option<Self> {
        SCALES
            .into_iter()
            .find(|s| s.title_text() == title || title == plain_title(*s))
    }
}

fn plain_title(s: Scale) -> &'static str {
    match s {
        Scale::Normal => "Normal Probability Plot",
        Scale::Weibull => "Weibull Probability Plot",
        _ => "",
    }
}

const LEVELS: [f64; 15] = [
    0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, 0.98, 0.99, 0.995, 0.999,
];

fn probability_label(p: f64) -> String {
    let s = format!("{p:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Sorted positions (x, p) of one column.
fn positions(s: &Sample) -> Vec<(f64, f64)> {
    if !s.has_cens() && !s.has_freq() {
        let mut x = s.x.clone();
        x.sort_by(f64::total_cmp);
        let n = x.len() as f64;
        return x
            .into_iter()
            .enumerate()
            .map(|(i, v)| (v, (i as f64 + 0.5) / n))
            .collect();
    }
    let km = kaplan_meier(s);
    let mut before = 0.;
    km.times
        .iter()
        .zip(&km.survival)
        .map(|(&t, &sv)| {
            let after = 1. - sv;
            let p = (before + after) / 2.;
            before = after;
            (t, p)
        })
        .collect()
}

/// Data quantile at probability p: the midpoint percentile for uncensored
/// data, the Kaplan-Meier inverse otherwise.
fn quantile(s: &Sample, sorted: &[f64], p: f64) -> f64 {
    if !s.has_cens() && !s.has_freq() {
        return percentile(sorted, p);
    }
    let km = kaplan_meier(s);
    for (t, sv) in km.times.iter().zip(&km.survival) {
        if 1. - sv >= p {
            return *t;
        }
    }
    *km.times.last().unwrap_or(&f64::NAN)
}

fn columns(e: &Engine, who: &str, v: &Value) -> Result<Vec<Vec<f64>>> {
    let m = e.real_arg(who, v)?;
    if m.is_nd() || m.is_empty() || v.is_character() {
        return Err(format!(
            "{who}: data must be a nonempty real vector or matrix"
        ));
    }
    let (n, cols) = if m.rows() == 1 {
        (m.cols(), 1)
    } else {
        (m.rows(), m.cols())
    };
    if cols > 16 {
        return Err(format!("{who}: at most 16 data columns are supported"));
    }
    Ok((0..cols)
        .map(|c| m.data()[c * n..(c + 1) * n].to_vec())
        .collect())
}

fn probplot(
    e: &mut Engine,
    fixed: Option<Scale>,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
    has_axes: bool,
) -> Result<Vec<Value>> {
    let who = match fixed {
        Some(Scale::Normal) => "normplot",
        Some(Scale::Weibull) => "wblplot",
        _ => "probplot",
    };
    let mut args = args.to_vec();
    let noref = matches!(args.last(), Some(Value::Text(t)) if t.eq_ignore_ascii_case("noref"));
    if noref {
        args.pop();
    }
    let mut scale = fixed.unwrap_or(Scale::Normal);
    let mut pd: Option<Distribution> = None;
    if fixed.is_none() {
        if let Some(Value::Text(name)) = args.first() {
            scale = Scale::parse(name)?;
            args.remove(0);
        } else if let Some(d) = args.first().and_then(|v| e.dso_get(v)) {
            args.remove(0);
            if d.kernel.is_some() {
                return Err("probplot: Kernel distributions have no probability scale".into());
            }
            scale = Scale::of_family(d.family).unwrap_or(Scale::Normal);
            pd = Some(d);
        }
    }
    if args.is_empty() {
        let Some(d) = pd else {
            return Err(format!("{who}: expected data"));
        };
        // probplot(ax, pd): add the fitted line to an existing plot.
        let title = e
            .figure
            .as_ref()
            .map(|f| f.axes.title.clone())
            .unwrap_or_default();
        let scale = Scale::from_title(&title).unwrap_or(scale);
        if !has_axes && e.figure.is_none() {
            return Err("probplot: no probability plot to add the line to".into());
        }
        let limits = e.call("xlim", vec![], out)?;
        let l = e.real_arg("probplot", &limits)?;
        let (a, b) = (l.data()[0], l.data()[1]);
        let line = held(e, out, |e, out| fitted_line(e, &d, scale, a, b, out))?;
        return Ok(if nargout > 0 { vec![line] } else { vec![] });
    }
    if args.len() > 3 {
        return Err(format!("{who}: too many input arguments"));
    }
    let data = columns(e, who, &args[0])?;
    let cens = match args.get(1) {
        Some(v) => columns(e, who, v)?,
        None => Vec::new(),
    };
    let freq = match args.get(2) {
        Some(v) => columns(e, who, v)?,
        None => Vec::new(),
    };
    for (what, v) in [("censoring", &cens), ("frequency", &freq)] {
        if !v.is_empty() && (v.len() != data.len() || v[0].len() != data[0].len()) {
            return Err(format!("{who}: the {what} data must match the size of y"));
        }
    }
    let mut samples = Vec::with_capacity(data.len());
    for (j, col) in data.iter().enumerate() {
        let c = cens.get(j).map(|v| column(v.clone()));
        let f = freq.get(j).map(|v| column(v.clone()));
        let s = sample::sample(e, who, &column(col.clone()), c.as_ref(), f.as_ref())?;
        if scale.log_x() && s.x.iter().any(|v| *v <= 0.) {
            return Err(format!(
                "{who}: {} probability plots need positive data",
                scale.title()
            ));
        }
        samples.push(s);
    }
    // Axis ranges over all columns.
    let mut all: Vec<(f64, f64)> = Vec::new();
    let mut per: Vec<Vec<(f64, f64)>> = Vec::new();
    for s in &samples {
        let p = positions(s);
        all.extend(p.iter().copied());
        per.push(p);
    }
    if all.is_empty() {
        return Err(format!("{who}: no uncensored observations to plot"));
    }
    let tx = |x: f64| if scale.log_x() { x.ln() } else { x };
    let ux = |t: f64| if scale.log_x() { t.exp() } else { t };
    let (xmin, xmax) = all
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
            (a.min(p.0), b.max(p.0))
        });
    let (pmin, pmax) = all
        .iter()
        .fold((1f64, 0f64), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let (t0, t1) = (tx(xmin), tx(xmax));
    let pad = if t1 > t0 { 0.05 * (t1 - t0) } else { 0.5 };
    let (xa, xb) = (ux(t0 - pad), ux(t1 + pad));
    let (y0, y1) = (scale.y(pmin), scale.y(pmax));
    let ypad = if y1 > y0 { 0.08 * (y1 - y0) } else { 0.5 };
    let (ya, yb) = (y0 - ypad, y1 + ypad);
    let mut handles = Vec::new();
    for (j, (s, p)) in samples.iter().zip(&per).enumerate() {
        let xs: Vec<f64> = p.iter().map(|q| q.0).collect();
        let ys: Vec<f64> = p.iter().map(|q| scale.y(q.1)).collect();
        let points = if j == 0 {
            e.call("plot", vec![column(xs), column(ys), text("b+")], out)?
        } else {
            held(e, out, |e, out| {
                e.call("plot", vec![column(xs), column(ys), text("b+")], out)
            })?
        };
        handles.push(points);
        if noref {
            continue;
        }
        let line = match &pd {
            Some(d) => held(e, out, |e, out| fitted_line(e, d, scale, xa, xb, out))?,
            None => {
                let mut sorted: Vec<f64> = s.iter().filter(|o| !o.1).map(|o| o.0).collect();
                sorted.sort_by(f64::total_cmp);
                let (pa, pb) = if scale == Scale::HalfNormal {
                    (0., 0.5)
                } else {
                    (0.25, 0.75)
                };
                let (qa, qb) = if scale == Scale::HalfNormal && !s.has_cens() && !s.has_freq() {
                    (sorted[0], percentile(&sorted, 0.5))
                } else {
                    (quantile(s, &sorted, pa), quantile(s, &sorted, pb))
                };
                let (ta, tb) = (tx(qa), tx(qb));
                let (ua, ub) = (scale.y(pa), scale.y(pb));
                let slope = if tb != ta { (ub - ua) / (tb - ta) } else { 0. };
                let at = |x: f64| ua + slope * (tx(x) - ta);
                held(e, out, |e, out| {
                    e.call(
                        "plot",
                        vec![row(vec![xa, xb]), row(vec![at(xa), at(xb)]), text("r--")],
                        out,
                    )
                })?
            }
        };
        handles.push(line);
    }
    // Probability labels at the transformed positions.
    held(e, out, |e, out| {
        let levels: Vec<f64> = LEVELS
            .into_iter()
            .filter(|p| (ya..=yb).contains(&scale.y(*p)))
            .collect();
        if !levels.is_empty() {
            let mut gx = Vec::new();
            let mut gy = Vec::new();
            for p in &levels {
                let y = scale.y(*p);
                gx.extend([xa, xb, f64::NAN]);
                gy.extend([y, y, f64::NAN]);
            }
            e.call(
                "plot",
                vec![
                    row(gx),
                    row(gy),
                    text(":"),
                    text("Color"),
                    row(vec![0.75, 0.75, 0.75]),
                ],
                out,
            )?;
            for p in &levels {
                let at = ux(tx(xa) + 0.01 * (tx(xb) - tx(xa)));
                e.call(
                    "text",
                    vec![
                        number(at),
                        number(scale.y(*p)),
                        text(&probability_label(*p)),
                    ],
                    out,
                )?;
            }
        }
        Ok(())
    })?;
    if scale.log_x() {
        let gca = e.call("gca", vec![], out)?;
        e.call("set", vec![gca, text("XScale"), text("log")], out)?;
    }
    e.call("xlim", vec![row(vec![xa, xb])], out)?;
    e.call("ylim", vec![row(vec![ya, yb])], out)?;
    let title = match fixed {
        Some(s) => plain_title(s).to_string(),
        None => scale.title_text(),
    };
    e.call("title", vec![text(&title)], out)?;
    e.call("xlabel", vec![text("Data")], out)?;
    e.call("ylabel", vec![text("Probability")], out)?;
    if nargout == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![e.call("vertcat", handles, out)?])
}

/// The fitted distribution drawn on the probability scale from a to b.
fn fitted_line(
    e: &mut Engine,
    d: &Distribution,
    scale: Scale,
    a: f64,
    b: f64,
    out: &mut Execution,
) -> Result<Value> {
    let n = 101;
    let log = scale.log_x() && a > 0.;
    let xs: Vec<f64> = (0..n)
        .map(|k| {
            let t = k as f64 / (n - 1) as f64;
            if log {
                (a.ln() + t * (b.ln() - a.ln())).exp()
            } else {
                a + t * (b - a)
            }
        })
        .collect();
    let ys: Vec<f64> = xs
        .iter()
        .map(|&x| {
            let p = d.family.cdf(x, &d.params, false);
            if p > 0. && p < 1. {
                scale.y(p)
            } else {
                f64::NAN
            }
        })
        .collect();
    e.call("plot", vec![row(xs), row(ys), text("r--")], out)
}

// ---------- qqplot ----------

fn qqplot(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let (xv, second) = match args {
        [x] => (x, None),
        [x, y] => (x, Some(y)),
        _ => return Err("qqplot: expected qqplot(x), qqplot(x, pd) or qqplot(x, y)".into()),
    };
    let x = sample::vector(e, "qqplot", "x", xv)?;
    let mut x: Vec<f64> = x.into_iter().filter(|v| !v.is_nan()).collect();
    if x.is_empty() {
        return Err("qqplot: x must contain at least one number".into());
    }
    x.sort_by(f64::total_cmp);
    let n = x.len();
    let probs: Vec<f64> = (0..n).map(|i| (i as f64 + 0.5) / n as f64).collect();
    // (horizontal, vertical) values and the two quartile points.
    let (hx, vy, labels, quart) = match second.map(|v| (e.dso_get(v), v)) {
        None => {
            let t: Vec<f64> = probs.iter().map(|&p| norm_inv(p)).collect();
            let q = (
                (norm_inv(0.25), percentile(&x, 0.25)),
                (norm_inv(0.75), percentile(&x, 0.75)),
            );
            (
                t,
                x.clone(),
                (
                    "Standard Normal Quantiles".to_string(),
                    "Quantiles of Input Sample".to_string(),
                    "QQ Plot of Sample Data versus Standard Normal".to_string(),
                ),
                q,
            )
        }
        Some((Some(d), _)) => {
            let icdf = |p: f64| -> Result<f64> {
                if let Some(k) = &d.kernel {
                    Ok(k.icdf(p))
                } else {
                    Ok(d.family.inv(p, &d.params))
                }
            };
            let t = probs.iter().map(|&p| icdf(p)).collect::<Result<Vec<_>>>()?;
            let q = (
                (icdf(0.25)?, percentile(&x, 0.25)),
                (icdf(0.75)?, percentile(&x, 0.75)),
            );
            (
                t,
                x.clone(),
                (
                    format!("Quantiles of {} Distribution", d.title()),
                    "Quantiles of Input Sample".to_string(),
                    format!("QQ Plot of Sample Data versus {}", d.title()),
                ),
                q,
            )
        }
        Some((None, yv)) => {
            let y = sample::vector(e, "qqplot", "y", yv)?;
            let mut y: Vec<f64> = y.into_iter().filter(|v| !v.is_nan()).collect();
            if y.is_empty() {
                return Err("qqplot: y must contain at least one number".into());
            }
            y.sort_by(f64::total_cmp);
            let interpolate = |sorted: &[f64], p: f64| {
                let m = sorted.len();
                let pos = p * m as f64 - 0.5;
                if pos <= 0. {
                    return sorted[0];
                }
                if pos >= (m - 1) as f64 {
                    return sorted[m - 1];
                }
                let k = pos.floor() as usize;
                let t = pos - k as f64;
                sorted[k] + t * (sorted[k + 1] - sorted[k])
            };
            let (hx, vy) = if x.len() == y.len() {
                (x.clone(), y.clone())
            } else if x.len() < y.len() {
                (
                    x.clone(),
                    probs.iter().map(|&p| interpolate(&y, p)).collect(),
                )
            } else {
                let m = y.len();
                let py: Vec<f64> = (0..m).map(|i| (i as f64 + 0.5) / m as f64).collect();
                (py.iter().map(|&p| interpolate(&x, p)).collect(), y.clone())
            };
            let q = (
                (percentile(&x, 0.25), percentile(&y, 0.25)),
                (percentile(&x, 0.75), percentile(&y, 0.75)),
            );
            (
                hx,
                vy,
                (
                    "X Quantiles".to_string(),
                    "Y Quantiles".to_string(),
                    "QQ Plot of Sample Data versus Sample Data".to_string(),
                ),
                q,
            )
        }
    };
    let ((a0, b0), (a1, b1)) = quart;
    let slope = if a1 != a0 { (b1 - b0) / (a1 - a0) } else { 0. };
    let line = |t: f64| b0 + slope * (t - a0);
    let lo = hx.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = hx.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let points = e.call("plot", vec![column(hx), column(vy), text("b+")], out)?;
    let (solid, dashed) = held(e, out, |e, out| {
        let solid = e.call(
            "plot",
            vec![row(vec![a0, a1]), row(vec![b0, b1]), text("r-")],
            out,
        )?;
        let dashed = e.call(
            "plot",
            vec![
                row(vec![lo, a0, f64::NAN, a1, hi]),
                row(vec![line(lo), b0, f64::NAN, b1, line(hi)]),
                text("r--"),
            ],
            out,
        )?;
        Ok((solid, dashed))
    })?;
    e.call("xlabel", vec![text(&labels.0)], out)?;
    e.call("ylabel", vec![text(&labels.1)], out)?;
    e.call("title", vec![text(&labels.2)], out)?;
    if nargout == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![e.call("vertcat", vec![points, solid, dashed], out)?])
}
