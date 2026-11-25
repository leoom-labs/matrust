//! fitdist, paramci, negloglik, mle, statset and the <family>fit and
//! <family>like functions.
use super::super::dist_objects::Distribution;
use super::super::stats_dist::families::Family;
use super::super::{Engine, Execution, Value};
use super::kernel::{KernelOptions, Smoother};
use super::mle::{self, Options};
use super::optimize::{self, Limits};
use super::sample::{self, Sample};
use crate::{Array, CellArray, CharacterRow, FunctionHandle, Matrix, Result, StructArray};
use std::sync::Arc;

fn max_outputs(name: &str) -> usize {
    match name {
        "fitdist" => 3,
        "paramci" | "negloglik" | "statset" => 1,
        _ => 2,
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if nargout > max_outputs(name) {
        return Err(format!("{name}: too many output arguments"));
    }
    match name {
        "fitdist" => fitdist(e, args, nargout, out),
        "paramci" | "negloglik" => {
            let Some(d) = args.first().and_then(|v| e.dso_get(v)) else {
                return Err(format!(
                    "{name}: the first argument must be a probability distribution object"
                ));
            };
            Ok(vec![super::object_method(e, &d, name, &args[1..])?])
        }
        "statset" => Ok(vec![statset(e, args)?]),
        "mle" => mle_call(e, args, nargout, out),
        "normlike" | "explike" | "gamlike" | "wbllike" | "lognlike" | "evlike" | "betalike" => {
            like(e, name, args, nargout)
        }
        "poissfit" => poissfit(e, args, nargout),
        "binofit" => binofit(e, args, nargout),
        _ => family_fit(e, name, args, nargout),
    }
}

fn row(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}

/// 2-by-k interval matrix.
fn intervals(ci: &[[f64; 2]]) -> Result<Value> {
    let data = ci.iter().flat_map(|c| [c[0], c[1]]).collect();
    Ok(Value::Matrix(Matrix::from_column_vec(2, ci.len(), data)?))
}

fn text_of(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

fn scalar_arg(e: &Engine, who: &str, what: &str, v: &Value) -> Result<f64> {
    if v.is_character() {
        return Err(format!("{who}: {what} must be a number"));
    }
    e.real_arg(who, v)?
        .as_scalar()
        .map_err(|_| format!("{who}: {what} must be a scalar"))
}

fn alpha_arg(e: &Engine, who: &str, v: Option<&Value>) -> Result<f64> {
    let Some(v) = v else {
        return Ok(0.05);
    };
    if let Value::Matrix(m) = v
        && m.is_empty()
    {
        return Ok(0.05);
    }
    let a = scalar_arg(e, who, "alpha", v)?;
    if a.is_nan() || a <= 0. || a >= 1. {
        return Err(format!("{who}: alpha must be between 0 and 1"));
    }
    Ok(a)
}

fn cell_column(e: &Engine, labels: &[String]) -> Result<Value> {
    let items = labels
        .iter()
        .map(|t| CharacterRow::from_text(t, &e.cancel).map(|c| Arc::new(Array::Character(c))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        items.len(),
        1,
        items,
        &e.cancel,
    )?)))
}

// ---------- fitdist ----------

struct FitRequest {
    family: Option<Family>,
    options: Options,
    kernel: KernelOptions,
    cens: Option<Value>,
    freq: Option<Value>,
    by: Option<Value>,
}

fn parse_fit_options(
    e: &Engine,
    who: &str,
    family: Option<Family>,
    pairs: &[Value],
    unbiased: bool,
) -> Result<FitRequest> {
    if !pairs.len().is_multiple_of(2) {
        return Err(format!("{who}: options must be name/value pairs"));
    }
    let mut r = FitRequest {
        family,
        options: Options::new(unbiased),
        kernel: KernelOptions {
            kind: Smoother::Normal,
            bandwidth: None,
            support: (f64::NEG_INFINITY, f64::INFINITY),
        },
        cens: None,
        freq: None,
        by: None,
    };
    let mut statset = None;
    let name = family.map_or("Kernel", mle::title);
    let applies = |ok: bool, key: &str| -> Result<()> {
        if ok {
            Ok(())
        } else {
            Err(format!(
                "{who}: option '{key}' does not apply to the {name} distribution"
            ))
        }
    };
    for pair in pairs.as_chunks::<2>().0 {
        let Some(key) = text_of(&pair[0]) else {
            return Err(format!("{who}: option names must be text"));
        };
        let value = &pair[1];
        match key.to_ascii_lowercase().as_str() {
            "censoring" => r.cens = Some(value.clone()),
            "frequency" => r.freq = Some(value.clone()),
            "options" => statset = Some(value.clone()),
            "by" if who == "fitdist" => r.by = Some(value.clone()),
            "ntrials" => {
                applies(family == Some(Family::Bino), key)?;
                let n = scalar_arg(e, who, "NTrials", value)?;
                if n < 1. || n.fract() != 0. || n >= 9_007_199_254_740_992. {
                    return Err(format!("{who}: NTrials must be a positive integer"));
                }
                r.options.ntrials = Some(n);
            }
            "theta" => {
                applies(family == Some(Family::Gp), key)?;
                let t = scalar_arg(e, who, "Theta", value)?;
                if !t.is_finite() {
                    return Err(format!("{who}: Theta must be finite"));
                }
                r.options.theta = t;
            }
            "mu" => {
                applies(family == Some(Family::HalfNorm), key)?;
                let m = scalar_arg(e, who, "mu", value)?;
                if !m.is_finite() {
                    return Err(format!("{who}: mu must be finite"));
                }
                r.options.mu = m;
            }
            "kernel" => {
                applies(family.is_none(), key)?;
                let Some(k) = text_of(value) else {
                    return Err(format!("{who}: 'Kernel' must be a kernel name"));
                };
                r.kernel.kind = Smoother::parse(who, k)?;
            }
            "bandwidth" | "width" => {
                applies(family.is_none(), key)?;
                let h = scalar_arg(e, who, "the bandwidth", value)?;
                if h.is_nan() || h <= 0. || !h.is_finite() {
                    return Err(format!(
                        "{who}: the bandwidth must be a positive finite number"
                    ));
                }
                r.kernel.bandwidth = Some(h);
            }
            "support" => {
                applies(family.is_none(), key)?;
                r.kernel.support = match text_of(value) {
                    Some(t) if t.eq_ignore_ascii_case("unbounded") => {
                        (f64::NEG_INFINITY, f64::INFINITY)
                    }
                    Some(t) if t.eq_ignore_ascii_case("positive") => (0., f64::INFINITY),
                    Some(t) => {
                        return Err(format!(
                            "{who}: 'Support' must be 'unbounded', 'positive' or [lower upper] (got '{t}')"
                        ));
                    }
                    None => {
                        let v = sample::vector(e, who, "'Support'", value)?;
                        if v.len() != 2
                            || v[0].partial_cmp(&v[1]) != Some(std::cmp::Ordering::Less)
                            || !v[0].is_finite()
                            || !v[1].is_finite()
                        {
                            return Err(format!(
                                "{who}: 'Support' bounds must be two finite increasing numbers"
                            ));
                        }
                        (v[0], v[1])
                    }
                };
            }
            _ => return Err(format!("{who}: unsupported option '{key}'")),
        }
    }
    let k = family.and_then(mle::kinds).map_or(1, |k| k.len());
    r.options.limits = sample::limits(e, who, statset.as_ref(), Limits::family(k))?;
    Ok(r)
}

fn family_or_kernel(who: &str, name: &str) -> Result<Option<Family>> {
    if name.eq_ignore_ascii_case("kernel") {
        return Ok(None);
    }
    let f = super::super::stats_dist::family_by_name(who, name)?;
    if mle::kinds(f).is_none() {
        return Err(format!(
            "{who}: fitting {} is not supported; supported: {}",
            mle::title(f),
            mle::FITTABLE
        ));
    }
    Ok(Some(f))
}

fn fit_sample(e: &mut Engine, r: &FitRequest, s: &Sample) -> Result<Distribution> {
    match r.family {
        None => {
            let k = super::kernel::fit("fitdist", s, &r.kernel)?;
            Ok(Distribution {
                family: Family::Norm,
                params: Vec::new(),
                fit: None,
                kernel: Some(Arc::new(k)),
            })
        }
        Some(f) => {
            if f == Family::Bino && r.options.ntrials.is_none() {
                return Err("fitdist: Binomial requires 'NTrials'".into());
            }
            let params = mle::fit(e, "fitdist", f, s, &r.options)?;
            super::fitted_object(e, f, params, s, mle::fixed(f))
        }
    }
}

fn fitdist(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if args.len() < 2 {
        return Err("fitdist: expected a data vector and distribution name".into());
    }
    let name = super::family_name("fitdist", &args[1])?;
    let family = family_or_kernel("fitdist", &name)?;
    let r = parse_fit_options(e, "fitdist", family, &args[2..], true)?;
    let Some(by) = r.by.clone() else {
        if nargout > 1 {
            return Err("fitdist: [pd, gn, gl] outputs need the 'By' option".into());
        }
        let s = sample::sample(e, "fitdist", &args[0], r.cens.as_ref(), r.freq.as_ref())?;
        let d = fit_sample(e, &r, &s)?;
        return Ok(vec![e.dso_new(d)?]);
    };
    let raw = e.real_arg("fitdist", &args[0])?;
    let (g, labels) = groups(e, &by, raw.len(), out)?;
    let full = |v: &Option<Value>| -> Result<Option<Vec<f64>>> {
        match v {
            None => Ok(None),
            Some(v) => {
                let d = sample::vector(e, "fitdist", "an option vector", v)?;
                if !d.is_empty() && d.len() != raw.len() {
                    return Err(
                        "fitdist: censoring and frequency vectors must have one entry per observation"
                            .into(),
                    );
                }
                Ok((!d.is_empty()).then_some(d))
            }
        }
    };
    let cens = full(&r.cens)?;
    let freq = full(&r.freq)?;
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); labels.len()];
    for (i, group) in g.iter().enumerate() {
        if let Some(k) = group {
            members[*k].push(i);
        }
    }
    let mut objects = Vec::with_capacity(labels.len());
    for (label, rows) in labels.iter().zip(&members) {
        e.check_cancel()?;
        let pick = |v: &[f64]| {
            let d: Vec<f64> = rows.iter().map(|&i| v[i]).collect();
            Value::Matrix(Matrix::from_column_vec(d.len(), 1, d).expect("column"))
        };
        let x = pick(raw.data());
        let c = cens.as_ref().map(|v| pick(v));
        let f = freq.as_ref().map(|v| pick(v));
        let s = sample::sample(e, "fitdist", &x, c.as_ref(), f.as_ref())
            .map_err(|m| format!("{m} (group '{label}')"))?;
        let d = fit_sample(e, &r, &s).map_err(|m| format!("{m} (group '{label}')"))?;
        objects.push(Arc::new(e.dso_new(d)?.array(&e.cancel)?));
    }
    let cell = Value::Container(Array::Cell(CellArray::from_shared(
        1,
        objects.len(),
        objects,
        &e.cancel,
    )?));
    let names = cell_column(e, &labels)?;
    Ok(vec![cell, names.clone(), names]
        .into_iter()
        .take(nargout.max(1))
        .collect())
}

/// Group of each observation (None when missing) and the group labels in
/// sorted order: numbers for a numeric grouping variable, text otherwise.
fn groups(
    e: &mut Engine,
    by: &Value,
    n: usize,
    out: &mut Execution,
) -> Result<(Vec<Option<usize>>, Vec<String>)> {
    let wrong =
        || "fitdist: the 'By' grouping variable must have one entry per observation".to_string();
    if matches!(by, Value::Matrix(_) | Value::Logical(_)) {
        let g = e.real_arg("fitdist", by)?;
        if g.len() != n {
            return Err(wrong());
        }
        let mut levels: Vec<f64> = g.data().iter().copied().filter(|v| !v.is_nan()).collect();
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        let index = g
            .data()
            .iter()
            .map(|v| levels.binary_search_by(|l| l.total_cmp(v)).ok())
            .collect();
        let labels = levels
            .iter()
            .map(|&v| super::super::string_class::number(v))
            .collect();
        return Ok((index, labels));
    }
    let strings = e.call("string", vec![by.clone()], out)?;
    let text: Vec<Option<String>> = match strings {
        Value::QuotedString(s) | Value::Text(s) => vec![Some(s)],
        Value::Container(Array::Str(s)) => s
            .data()
            .iter()
            .map(|t| t.as_deref().map(str::to_string))
            .collect(),
        _ => return Err("fitdist: unsupported 'By' grouping variable".into()),
    };
    if text.len() != n {
        return Err(wrong());
    }
    let mut levels: Vec<String> = text.iter().flatten().cloned().collect();
    levels.sort();
    levels.dedup();
    let index = text
        .iter()
        .map(|t| t.as_ref().and_then(|t| levels.binary_search(t).ok()))
        .collect();
    Ok((index, levels))
}

// ---------- paramci ----------

pub(super) fn paramci(e: &mut Engine, d: &Distribution, rest: &[Value]) -> Result<Value> {
    if d.kernel.is_some() {
        return Err("paramci: Kernel distributions have no parameters".into());
    }
    if !rest.len().is_multiple_of(2) {
        return Err("paramci: options must be name/value pairs".into());
    }
    let mut alpha = 0.05;
    let mut pick: Option<Vec<usize>> = None;
    let names = d.names();
    for pair in rest.as_chunks::<2>().0 {
        let Some(key) = text_of(&pair[0]) else {
            return Err("paramci: option names must be text".into());
        };
        match key.to_ascii_lowercase().as_str() {
            "alpha" => alpha = alpha_arg(e, "paramci", Some(&pair[1]))?,
            "parameter" => {
                let wanted: Vec<String> = match &pair[1] {
                    Value::Text(t) | Value::QuotedString(t) => vec![t.clone()],
                    Value::Container(Array::Cell(c)) => (0..c.len())
                        .map(|i| e.text_value(Value::from(c.get_linear(i)?.as_ref().clone())))
                        .collect::<Result<_>>()?,
                    v => {
                        let idx = sample::vector(e, "paramci", "'Parameter'", v)?;
                        let mut out = Vec::new();
                        for i in idx {
                            if i < 1. || i.fract() != 0. || i as usize > names.len() {
                                return Err("paramci: parameter index out of range".into());
                            }
                            out.push(names[i as usize - 1].to_string());
                        }
                        out
                    }
                };
                let mut idx = Vec::new();
                for w in wanted {
                    idx.push(
                        names
                            .iter()
                            .position(|n| n.eq_ignore_ascii_case(&w))
                            .ok_or_else(|| {
                                format!(
                                    "paramci: unknown parameter '{w}'; expected {}",
                                    names.join(", ")
                                )
                            })?,
                    );
                }
                pick = Some(idx);
            }
            _ => return Err(format!("paramci: unsupported option '{key}'")),
        }
    }
    let ci: Vec<[f64; 2]> = match &d.fit {
        Some(fit) => mle::param_ci(
            d.family,
            &d.params,
            &fit.sample(),
            &fit.cov,
            &fit.fixed,
            alpha,
        ),
        None => d.params.iter().map(|&v| [v, v]).collect(),
    };
    let ci: Vec<[f64; 2]> = match pick {
        Some(idx) => idx.iter().map(|&i| ci[i]).collect(),
        None => ci,
    };
    intervals(&ci)
}

// ---------- statset ----------

const STATSET_FIELDS: &[&str] = &[
    "Display",
    "MaxFunEvals",
    "MaxIter",
    "TolBnd",
    "TolFun",
    "TolX",
    "DerivStep",
    "FunValCheck",
    "GradObj",
    "Jacobian",
    "OutputFcn",
    "Robust",
    "Tune",
    "UseParallel",
];

fn statset(e: &Engine, args: &[Value]) -> Result<Value> {
    let empty = || {
        Arc::new(Array::Numeric(
            Matrix::from_column_vec(0, 0, vec![]).expect("empty"),
        ))
    };
    let mut slots: Vec<Arc<Array>> = STATSET_FIELDS.iter().map(|_| empty()).collect();
    let mut rest = args;
    let merge = |s: &StructArray, slots: &mut Vec<Arc<Array>>| -> Result<()> {
        for (i, name) in s.field_names().iter().enumerate() {
            let k = STATSET_FIELDS
                .iter()
                .position(|f| f.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("statset: unknown parameter name '{name}'"))?;
            slots[k] = s.get(i, 0)?.clone();
        }
        Ok(())
    };
    while let Some(Value::Container(Array::Struct(s))) = rest.first() {
        if s.len() != 1 {
            return Err("statset: options must be a scalar structure".into());
        }
        merge(s, &mut slots)?;
        rest = &rest[1..];
    }
    if !rest.len().is_multiple_of(2) {
        return Err("statset: arguments must be name/value pairs".into());
    }
    for pair in rest.as_chunks::<2>().0 {
        let Some(key) = text_of(&pair[0]) else {
            return Err("statset: parameter names must be text".into());
        };
        let k = STATSET_FIELDS
            .iter()
            .position(|f| f.eq_ignore_ascii_case(key))
            .ok_or_else(|| format!("statset: unknown parameter name '{key}'"))?;
        slots[k] = Arc::new(pair[1].clone().array(&e.cancel)?);
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        STATSET_FIELDS.iter().map(|s| (*s).to_string()).collect(),
        slots,
        &e.cancel,
    )?)))
}

// ---------- <family>like ----------

fn like(e: &mut Engine, name: &str, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    let f = match name {
        "normlike" => Family::Norm,
        "explike" => Family::Exp,
        "gamlike" => Family::Gam,
        "wbllike" => Family::Wbl,
        "lognlike" => Family::Logn,
        "evlike" => Family::Ev,
        _ => Family::Beta,
    };
    let max_args = if f == Family::Beta { 2 } else { 4 };
    if args.len() < 2 || args.len() > max_args {
        return Err(format!(
            "{name}: expected parameters, data{}",
            if max_args == 4 {
                " and optional censoring and frequency vectors"
            } else {
                ""
            }
        ));
    }
    let params = sample::vector(e, name, "the parameters", &args[0])?;
    let k = f.defaults().len();
    if params.len() != k {
        return Err(format!(
            "{name}: expected {k} parameter{}",
            if k == 1 { "" } else { "s" }
        ));
    }
    let s = sample::sample(e, name, &args[1], args.get(2), args.get(3))?;
    let nll = mle::nll(f, &params, &s);
    let mut outputs = vec![Value::Matrix(Matrix::scalar(nll))];
    if nargout > 1 {
        let cov = if f.valid(&params) {
            mle::covariance(e, f, &params, &s, &vec![false; k])?
        } else {
            vec![f64::NAN; k * k]
        };
        outputs.push(Value::Matrix(Matrix::from_column_vec(k, k, cov)?));
    }
    Ok(outputs)
}

// ---------- <family>fit ----------

fn family_fit(e: &mut Engine, name: &str, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    // (family, has censoring/frequency arguments, index of the options argument)
    let (f, cens_freq, options) = match name {
        "gamfit" => (Family::Gam, true, Some(4)),
        "wblfit" => (Family::Wbl, true, Some(4)),
        "lognfit" => (Family::Logn, true, Some(4)),
        "evfit" => (Family::Ev, true, Some(4)),
        "raylfit" => (Family::Rayl, true, None),
        "betafit" => (Family::Beta, false, None),
        "nbinfit" => (Family::Nbin, false, Some(2)),
        "gevfit" => (Family::Gev, false, Some(2)),
        _ => (Family::Gp, false, Some(2)),
    };
    let max_args = options.map_or(if cens_freq { 4 } else { 2 }, |i| i + 1);
    if args.is_empty() || args.len() > max_args {
        return Err(format!(
            "{name}: expected the data and at most {} more arguments",
            max_args - 1
        ));
    }
    let alpha = alpha_arg(e, name, args.get(1))?;
    let (cens, freq) = if cens_freq {
        (args.get(2), args.get(3))
    } else {
        (None, None)
    };
    let s = sample::sample(e, name, &args[0], cens, freq)?;
    let mut o = Options::new(true);
    o.limits = sample::limits(
        e,
        name,
        options.and_then(|i| args.get(i)),
        Limits::family(3),
    )?;
    let q = mle::fit(e, name, f, &s, &o)?;
    let fixed = mle::fixed(f);
    let free: Vec<usize> = (0..q.len()).filter(|&i| !fixed[i]).collect();
    let mut outputs = vec![row(free.iter().map(|&i| q[i]).collect())?];
    if nargout > 1 {
        let cov = mle::covariance(e, f, &q, &s, &fixed)?;
        let ci = mle::param_ci(f, &q, &s, &cov, &fixed, alpha);
        outputs.push(intervals(&free.iter().map(|&i| ci[i]).collect::<Vec<_>>())?);
    }
    Ok(outputs)
}

/// `[lambdahat, lambdaci] = poissfit(x, alpha)`, column-wise for a matrix.
fn poissfit(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    if args.is_empty() || args.len() > 2 {
        return Err("poissfit: expected the data and an optional alpha".into());
    }
    let alpha = alpha_arg(e, "poissfit", args.get(1))?;
    let x = e.real_arg("poissfit", &args[0])?;
    if x.is_nd() || x.is_empty() || args[0].is_character() {
        return Err("poissfit: data must be a nonempty real vector or matrix".into());
    }
    let (n, columns) = if x.rows() == 1 {
        (x.cols(), 1)
    } else {
        (x.rows(), x.cols())
    };
    let mut est = Vec::with_capacity(columns);
    let mut ci = Vec::with_capacity(columns);
    for c in 0..columns {
        e.check_cancel()?;
        let col = x.data()[c * n..(c + 1) * n].to_vec();
        if col
            .iter()
            .any(|v| *v < 0. || v.fract() != 0. || !v.is_finite())
        {
            return Err(
                "poissfit: observations must be integer counts within the distribution support"
                    .into(),
            );
        }
        let s = Sample::plain(col);
        let q = vec![s.iter().map(|o| o.0).sum::<f64>() / n as f64];
        est.push(q[0]);
        ci.push(mle::param_ci(Family::Poiss, &q, &s, &[0.], &[false], alpha)[0]);
    }
    let mut outputs = vec![row(est)?];
    if nargout > 1 {
        outputs.push(intervals(&ci)?);
    }
    Ok(outputs)
}

/// `[phat, pci] = binofit(x, n, alpha)`: each element of x separately.
fn binofit(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    if args.len() < 2 || args.len() > 3 {
        return Err("binofit: expected successes x, trials n and an optional alpha".into());
    }
    let alpha = alpha_arg(e, "binofit", args.get(2))?;
    let x = e.real_arg("binofit", &args[0])?;
    let n = e.real_arg("binofit", &args[1])?;
    if x.is_empty() || (n.len() != 1 && n.len() != x.len()) {
        return Err("binofit: n must be a scalar or match the size of x".into());
    }
    e.preflight_elements(x.len().saturating_mul(3))?;
    let mut phat = Vec::with_capacity(x.len());
    let mut lower = Vec::with_capacity(x.len());
    let mut upper = Vec::with_capacity(x.len());
    for (i, &k) in x.data().iter().enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let m = if n.len() == 1 {
            n.data()[0]
        } else {
            n.data()[i]
        };
        if m.is_nan() || m < 1. || m.fract() != 0. || !m.is_finite() {
            return Err("binofit: the number of trials must be a positive integer".into());
        }
        if !(0. ..=m).contains(&k) || k.fract() != 0. {
            return Err("binofit: x must be integer counts between 0 and n".into());
        }
        let q = [m, k / m];
        let ci = mle::param_ci(
            Family::Bino,
            &q,
            &Sample::plain(vec![k]),
            &[0.; 4],
            &[true, false],
            alpha,
        );
        phat.push(k / m);
        lower.push(ci[1][0]);
        upper.push(ci[1][1]);
    }
    let dims = x.dims().into_owned();
    let mut outputs = vec![Value::Matrix(
        Matrix::from_column_vec(dims[0], dims[1..].iter().product(), phat)?.with_dims(&dims)?,
    )];
    if nargout > 1 {
        let count = lower.len();
        lower.extend(upper);
        outputs.push(Value::Matrix(Matrix::from_column_vec(count, 2, lower)?));
    }
    Ok(outputs)
}

// ---------- mle ----------

struct Custom {
    pdf: Option<FunctionHandle>,
    cdf: Option<FunctionHandle>,
    logpdf: Option<FunctionHandle>,
    logsf: Option<FunctionHandle>,
    nloglf: Option<FunctionHandle>,
}

fn handle(who: &str, key: &str, v: &Value) -> Result<FunctionHandle> {
    match v {
        Value::Container(Array::Handle(h)) => Ok(h.clone()),
        _ => Err(format!("{who}: '{key}' must be a function handle")),
    }
}

fn mle_call(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let Some(data) = args.first() else {
        return Err("mle: expected the data".into());
    };
    let rest = &args[1..];
    if !rest.len().is_multiple_of(2) {
        return Err("mle: options must be name/value pairs".into());
    }
    let mut dist = "normal".to_string();
    let mut custom = Custom {
        pdf: None,
        cdf: None,
        logpdf: None,
        logsf: None,
        nloglf: None,
    };
    let mut alpha = 0.05;
    let mut start: Option<Vec<f64>> = None;
    let mut lower: Option<Vec<f64>> = None;
    let mut upper: Option<Vec<f64>> = None;
    let mut fit_pairs = Vec::new();
    for pair in rest.as_chunks::<2>().0 {
        let Some(key) = text_of(&pair[0]) else {
            return Err("mle: option names must be text".into());
        };
        let v = &pair[1];
        match key.to_ascii_lowercase().as_str() {
            "distribution" => {
                dist = text_of(v)
                    .ok_or("mle: 'Distribution' must be a distribution name")?
                    .to_string();
            }
            "pdf" => custom.pdf = Some(handle("mle", key, v)?),
            "cdf" => custom.cdf = Some(handle("mle", key, v)?),
            "logpdf" => custom.logpdf = Some(handle("mle", key, v)?),
            "logsf" => custom.logsf = Some(handle("mle", key, v)?),
            "nloglf" => custom.nloglf = Some(handle("mle", key, v)?),
            "alpha" => alpha = alpha_arg(e, "mle", Some(v))?,
            "start" => start = Some(sample::vector(e, "mle", "'Start'", v)?),
            "lowerbound" => lower = Some(sample::vector(e, "mle", "'LowerBound'", v)?),
            "upperbound" => upper = Some(sample::vector(e, "mle", "'UpperBound'", v)?),
            "optimfun" => {
                let name = text_of(v).unwrap_or("");
                if !name.eq_ignore_ascii_case("fminsearch") {
                    return Err("mle: only 'OptimFun','fminsearch' is supported".into());
                }
            }
            _ => fit_pairs.extend_from_slice(pair),
        }
    }
    let is_custom = custom.pdf.is_some()
        || custom.logpdf.is_some()
        || custom.nloglf.is_some()
        || custom.cdf.is_some()
        || custom.logsf.is_some();
    if is_custom {
        return mle_custom(
            e, data, &custom, start, lower, upper, alpha, &fit_pairs, nargout, out,
        );
    }
    if start.is_some() || lower.is_some() || upper.is_some() {
        return Err(
            "mle: 'Start', 'LowerBound' and 'UpperBound' apply to custom distributions only".into(),
        );
    }
    let family = family_or_kernel("mle", &dist)?
        .ok_or("mle: the Kernel distribution has no parameters to estimate")?;
    let r = parse_fit_options(e, "mle", Some(family), &fit_pairs, false)?;
    let s = sample::sample(e, "mle", data, r.cens.as_ref(), r.freq.as_ref())?;
    if family == Family::Bino && r.options.ntrials.is_none() {
        return Err("mle: Binomial requires 'NTrials'".into());
    }
    let q = mle::fit(e, "mle", family, &s, &r.options)?;
    let fixed = mle::fixed(family);
    let free: Vec<usize> = (0..q.len()).filter(|&i| !fixed[i]).collect();
    let mut outputs = vec![row(free.iter().map(|&i| q[i]).collect())?];
    if nargout > 1 {
        // Normal and lognormal intervals are the exact ones of normfit and
        // lognfit, which use the unbiased sigma.
        let at = if matches!(family, Family::Norm | Family::Logn) && !s.has_cens() {
            let mut o = r.options;
            o.unbiased = true;
            mle::fit(e, "mle", family, &s, &o)?
        } else {
            q.clone()
        };
        let cov = mle::covariance(e, family, &at, &s, &fixed)?;
        let ci = mle::param_ci(family, &at, &s, &cov, &fixed, alpha);
        outputs.push(intervals(&free.iter().map(|&i| ci[i]).collect::<Vec<_>>())?);
    }
    Ok(outputs)
}

#[allow(clippy::too_many_arguments)]
fn mle_custom(
    e: &mut Engine,
    data: &Value,
    c: &Custom,
    start: Option<Vec<f64>>,
    lower: Option<Vec<f64>>,
    upper: Option<Vec<f64>>,
    alpha: f64,
    pairs: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let start = start.ok_or("mle: custom distributions need 'Start' values")?;
    let k = start.len();
    if k == 0 || k > 64 {
        return Err("mle: 'Start' must have between 1 and 64 values".into());
    }
    let lower = lower.unwrap_or_else(|| vec![f64::NEG_INFINITY; k]);
    let upper = upper.unwrap_or_else(|| vec![f64::INFINITY; k]);
    if lower.len() != k || upper.len() != k {
        return Err("mle: 'LowerBound' and 'UpperBound' must match the size of 'Start'".into());
    }
    if start
        .iter()
        .zip(lower.iter().zip(&upper))
        .any(|(s, (l, u))| !(s >= l && s <= u) || !s.is_finite())
    {
        return Err("mle: 'Start' values must be finite and within the bounds".into());
    }
    let mut cens = None;
    let mut freq = None;
    let mut statset = None;
    for pair in pairs.as_chunks::<2>().0 {
        let key = text_of(&pair[0]).unwrap_or("");
        match key.to_ascii_lowercase().as_str() {
            "censoring" => cens = Some(pair[1].clone()),
            "frequency" => freq = Some(pair[1].clone()),
            "options" => statset = Some(pair[1].clone()),
            _ => return Err(format!("mle: unsupported option '{key}'")),
        }
    }
    let s = sample::sample(e, "mle", data, cens.as_ref(), freq.as_ref())?;
    if s.has_cens() && c.nloglf.is_none() && c.cdf.is_none() && c.logsf.is_none() {
        return Err("mle: censored data need a 'cdf' or 'logsf' function (or 'nloglf')".into());
    }
    if c.nloglf.is_none() && c.pdf.is_none() && c.logpdf.is_none() {
        return Err("mle: give 'pdf', 'logpdf' or 'nloglf' for a custom distribution".into());
    }
    let limits = sample::limits(e, "mle", statset.as_ref(), Limits::custom(k))?;
    let column =
        |v: Vec<f64>| Value::Matrix(Matrix::from_column_vec(v.len(), 1, v).expect("column"));
    let exact: Vec<f64> = s.iter().filter(|o| !o.1).map(|o| o.0).collect();
    let exact_w: Vec<f64> = s.iter().filter(|o| !o.1).map(|o| o.2).collect();
    let censored: Vec<f64> = s.iter().filter(|o| o.1).map(|o| o.0).collect();
    let censored_w: Vec<f64> = s.iter().filter(|o| o.1).map(|o| o.2).collect();
    let all = column(s.x.clone());
    let cens_value = column(s.cens.iter().map(|&b| f64::from(u8::from(b))).collect());
    let freq_value = column(s.freq.clone());
    let exact_value = column(exact.clone());
    let censored_value = column(censored.clone());
    let mut objective = |p: &[f64]| -> Result<f64> {
        e.check_cancel()?;
        if p.iter()
            .zip(lower.iter().zip(&upper))
            .any(|(v, (l, u))| v < l || v > u)
        {
            return Ok(f64::INFINITY);
        }
        let params: Vec<Value> = p
            .iter()
            .map(|&v| Value::Matrix(Matrix::scalar(v)))
            .collect();
        let mut eval = |h: &FunctionHandle, x: &Value| -> Result<Vec<f64>> {
            let mut a = vec![x.clone()];
            a.extend(params.iter().cloned());
            let v = e.call_handle(h, a, 1, None, out)?;
            let v = v.into_iter().next().unwrap_or(Value::Void);
            Ok(e.real_arg("mle", &v)?.data().to_vec())
        };
        let total = if let Some(h) = &c.nloglf {
            let row = Value::Matrix(Matrix::from_column_vec(1, p.len(), p.to_vec())?);
            let v = e.call_handle(
                h,
                vec![row, all.clone(), cens_value.clone(), freq_value.clone()],
                1,
                None,
                out,
            )?;
            let v = v.into_iter().next().unwrap_or(Value::Void);
            e.real_arg("mle", &v)?
                .as_scalar()
                .map_err(|_| "mle: 'nloglf' must return a scalar".to_string())?
        } else {
            let mut total = 0.;
            if !exact.is_empty() {
                let (h, log) = match (&c.logpdf, &c.pdf) {
                    (Some(h), _) => (h, true),
                    (None, Some(h)) => (h, false),
                    _ => unreachable!(),
                };
                let v = eval(h, &exact_value)?;
                if v.len() != exact.len() {
                    return Err("mle: the density must return one value per observation".into());
                }
                for (d, w) in v.iter().zip(&exact_w) {
                    total -= w * if log { *d } else { d.ln() };
                }
            }
            if !censored.is_empty() {
                let (h, log) = match (&c.logsf, &c.cdf) {
                    (Some(h), _) => (h, true),
                    (None, Some(h)) => (h, false),
                    _ => unreachable!(),
                };
                let v = eval(h, &censored_value)?;
                if v.len() != censored.len() {
                    return Err("mle: the cdf must return one value per observation".into());
                }
                for (d, w) in v.iter().zip(&censored_w) {
                    total -= w * if log { *d } else { (1. - d).ln() };
                }
            }
            total
        };
        Ok(if total.is_nan() { f64::INFINITY } else { total })
    };
    let step: Vec<f64> = start
        .iter()
        .map(|v| if *v != 0. { 0.05 * v } else { 0.00025 })
        .collect();
    let (p, fp) = optimize::nelder_mead(&mut objective, &start, &step, &limits)
        .map_err(|m| format!("mle: {m}"))?;
    let h: Vec<f64> = p.iter().map(|v| 1e-4 * v.abs().max(1e-2)).collect();
    let (p, _) = optimize::polish(&mut objective, p, fp, &h)?;
    let mut outputs = vec![row(p.clone())?];
    if nargout > 1 {
        let hess = optimize::hessian(&mut objective, &p, &h)?;
        let cov = optimize::inverse(k, &hess).unwrap_or_else(|| vec![f64::NAN; k * k]);
        let z = Family::Norm.inv(1. - alpha / 2., &[0., 1.]);
        let ci: Vec<[f64; 2]> = (0..k)
            .map(|i| {
                let se = cov[i * k + i].sqrt();
                [p[i] - z * se, p[i] + z * se]
            })
            .collect();
        outputs.push(intervals(&ci)?);
    }
    Ok(outputs)
}
