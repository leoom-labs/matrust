//! ARIMA/conditional-variance composition. Specification: MathWorks public
//! arima.{estimate,infer,forecast,simulate,summarize}, Name-Value (Y0/E0/V0,
//! Variance0), EstParamCov and Algorithms sections. References: composite.py.
use super::super::econ_vol::{self as vol, Model as VolModel};
use super::*;
use likelihood::{
    clean_aligned, optional_matrix, presample_column, sparse, tail_matrix, validate_pre,
};

#[derive(Clone)]
pub(in crate::engine) struct Variance {
    pub model: Arc<VolModel>,
    // Retain and trace the value object, so read-only property access needs no
    // registration or deep copy. The numerical model is immutable shared state.
    pub object: Array,
}

pub(in crate::engine) fn sync_variance(e: &mut Engine, m: &mut Model) -> Result<()> {
    if let Some(v) = &mut m.conditional {
        if v.model.offset != 0. {
            return Err(err(
                "arima",
                "InvalidOffset",
                "The conditional variance model Offset must be 0.",
            ));
        }
        let same = match (v.model.dof, m.dof) {
            (None, None) => true,
            (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
            _ => false,
        };
        if !same {
            let model = Arc::make_mut(&mut v.model);
            model.dof = m.dof;
            model.fit = None;
            v.object = super::super::objects::econ_vol_new(e, model.clone())?.array(&e.cancel)?;
        }
        v.model.validate()?;
    }
    Ok(())
}

pub(super) fn assign_variance(
    e: &mut Engine,
    variance: &Variance,
    steps: &[crate::syntax::PathStep],
    value: Array,
    out: &mut Execution,
) -> Result<Array> {
    let changed = vol::model_assign(e, &variance.model, steps, value, out)?;
    super::super::objects::econ_vol_new(e, changed)?.array(&e.cancel)
}

// Align all presamples before deleting missing rows, including X0 for forecast.
fn presamples(e: &Engine, o: &BTreeMap<String, Value>) -> Result<Vec<Option<Matrix>>> {
    clean_aligned(
        e,
        ["Y0", "E0", "V0", "X0"]
            .into_iter()
            .map(|n| optional_matrix(e, o, n))
            .collect::<Result<_>>()?,
    )
}

fn check_pre(m: &Option<Matrix>, lag: usize, paths: usize, name: &str, exact: bool) -> Result<()> {
    if let Some(m) = m {
        if exact && m.cols() != 1 && m.cols() != paths {
            return Err(err(
                "forecast",
                "InvalidPresample",
                "Presamples must have one column or the same number of paths.",
            ));
        }
        if name == "V0" && m.data().iter().any(|v| *v <= 0.) {
            return Err(err(
                "arima",
                "InvalidPresample",
                "V0 conditional variances must be positive.",
            ));
        }
    }
    validate_pre(m, lag, paths, name)?;
    Ok(())
}

fn full_column(m: &Matrix, path: usize) -> &[f64] {
    let j = if m.cols() == 1 { 0 } else { path };
    &m.data()[j * m.rows()..(j + 1) * m.rows()]
}

pub(super) fn infer(e: &Engine, model: &Model, a: &[Value]) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(err(
            "infer",
            "InvalidNumInputs",
            "Not enough input arguments.",
        ));
    }
    let mut o = options(e, "infer", &a[1..], &["Y0", "E0", "V0", "X"])?;
    let mut m = model.clone();
    if !o.contains_key("X") {
        m.beta.clear();
    }
    m.fully_specified()?;
    let vm = &m.conditional.as_ref().unwrap().model;
    let mut pre = presamples(e, &o)?;
    let v0 = pre.remove(2);
    // A supplied history emptied by listwise deletion is insufficient, rather
    // than an instruction to switch to the omitted-presample defaults.
    check_pre(&pre[0], m.p(), 1, "Y0", false)?;
    check_pre(&pre[1], m.q().max(vm.q()), 1, "E0", false)?;
    // Use precisely the cleaned presamples for both the mean and variance.
    for (name, value) in ["Y0", "E0"].into_iter().zip(pre) {
        o.remove(name);
        if let Some(value) = value {
            o.insert(name.into(), Value::Matrix(value));
        }
    }
    o.remove("V0");
    let s = likelihood::sample(e, &m, &a[0], &o)?;
    let n = s.y.rows();
    let paths = s.y.cols();
    preflight(e, n, paths, 7)?;
    check_pre(&s.e0, m.q().max(vm.q()), paths, "E0", false)?;
    check_pre(&v0, vm.variance_presample(), paths, "V0", false)?;
    let mut es = Vec::with_capacity(n * paths);
    let mut vs = Vec::with_capacity(n * paths);
    let mut ll = Vec::with_capacity(paths);
    for j in 0..paths {
        let z = likelihood::residuals(e, &m, &s, j)?;
        // arima.infer E0 defaults to zero even for GARCH/GJR; vol.infer's
        // second-moment E0 default is deliberately bypassed.
        let ep = presample_column(&s.e0, j, vm.q(), 0.);
        let vp = v0.as_ref().map(|v| full_column(v, j));
        let v = vol::conditional_variance(vm, &z, Some(&ep), vp, &e.cancel)?;
        ll.push(log_likelihood(e, m.dof, &z, &v)?);
        es.extend(z);
        vs.extend(v);
    }
    Ok(vec![
        value_matrix(n, paths, es)?,
        value_matrix(n, paths, vs)?,
        value_matrix(1, paths, ll)?,
    ])
}

fn log_likelihood(e: &Engine, dof: Option<f64>, z: &[f64], v: &[f64]) -> Result<f64> {
    let constant = dof.map_or(-0.5 * std::f64::consts::TAU.ln(), |nu| {
        libm::lgamma((nu + 1.) / 2.)
            - libm::lgamma(nu / 2.)
            - 0.5 * (std::f64::consts::PI * (nu - 2.)).ln()
    });
    let mut ll = 0.;
    for (i, (&z, &v)) in z.iter().zip(v).enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        ll += constant
            - 0.5 * v.ln()
            - dof.map_or(0.5 * z * z / v, |nu| {
                0.5 * (nu + 1.) * (z * z / ((nu - 2.) * v)).ln_1p()
            });
    }
    if !ll.is_finite() {
        return Err(err(
            "infer",
            "NumericalFailure",
            "Composite loglikelihood overflowed.",
        ));
    }
    Ok(ll)
}

// likelihood::sample cleans Y0/E0 with V0 before selecting their tails. Keep
// that same alignment here, and validate the larger volatility-model degrees.
pub(super) fn estimation_presample(
    e: &Engine,
    m: &Model,
    o: &BTreeMap<String, Value>,
) -> Result<Option<Matrix>> {
    let mut pre = presamples(e, o)?;
    let vm = &m.conditional.as_ref().unwrap().model;
    for (p, name, lag) in [
        (&pre[0], "Y0", m.p()),
        (&pre[1], "E0", m.q().max(vm.q())),
        (&pre[2], "V0", vm.variance_presample()),
    ] {
        if p.as_ref().is_some_and(|p| p.cols() != 1) {
            return Err(err(
                "estimate",
                "InvalidPresample",
                "Estimation presamples must be column vectors.",
            ));
        }
        check_pre(p, lag, 1, name, true)?;
    }
    Ok(pre.remove(2))
}

pub(super) fn estimation_contributions(
    e: &Engine,
    m: &Model,
    s: &likelihood::Sample,
    v0: &Option<Matrix>,
) -> Result<Vec<f64>> {
    let z = likelihood::residuals(e, m, s, 0)?;
    let vm = &m.conditional.as_ref().unwrap().model;
    let ep = presample_column(&s.e0, 0, vm.q(), 0.);
    // Omitted V0 is recomputed from the current inferred residuals at every
    // likelihood evaluation; unconditional variance is a simulation default.
    let v = vol::estimation_variance(vm, &z, Some(&ep), v0.as_ref().map(Matrix::data), &e.cancel)?;
    vol::likelihood_contributions(vm, &z, &v, &e.cancel)
}

pub(super) fn initial_parameters(
    e: &Engine,
    m: &Model,
    s: &likelihood::Sample,
) -> Result<Vec<f64>> {
    let mut mean = m.clone();
    mean.conditional = None;
    let mut p = estimate::initial_parameters(e, &mean, s)?;
    let nu = m.dof.map(|_| p.pop().unwrap());
    p.pop(); // The constant-variance start is replaced by the variance block.
    let mut mean_params = p.clone();
    mean_params.push(1.);
    mean_params.extend(nu);
    let z = likelihood::residuals(e, &mean.with_parameters(&mean_params), s, 0)?;
    let moment = (z.iter().map(|v| v * v).sum::<f64>() / z.len() as f64).max(1e-8);
    let vm = vol::estimation_start(&m.conditional.as_ref().unwrap().model, moment, 0.);
    let mut vp = vol::initial_values(&m.conditional.as_ref().unwrap().model, &vm);
    if vm.dof.is_some() {
        vp.pop();
    }
    p.extend(vp);
    p.extend(nu);
    Ok(p)
}

pub(super) fn starting_variance(e: &Engine, m: &Model, value: &Value, p: &mut [f64]) -> Result<()> {
    let Array::Cell(c) = value.clone().array(&e.cancel)? else {
        return Err(err(
            "estimate",
            "InvalidInitialParameter",
            "Composite Variance0 must be a cell vector of starting-value name-value arguments.",
        ));
    };
    if c.rows() > 1 && c.cols() > 1 {
        return Err(err(
            "estimate",
            "InvalidInitialParameter",
            "Variance0 must be a cell vector.",
        ));
    }
    let a = (0..c.len())
        .map(|i| Ok(Value::from(c.get_linear(i)?.as_ref().clone())))
        .collect::<Result<Vec<_>>>()?;
    let vm = &m.conditional.as_ref().unwrap().model;
    let allowed = if vm.kind == vol::Kind::Garch {
        vec!["Constant0", "GARCH0", "ARCH0"]
    } else {
        vec!["Constant0", "GARCH0", "ARCH0", "Leverage0"]
    };
    let opts = options(e, "estimate", &a, &allowed)?;
    let names = m.parameter_names();
    let original = m.parameters();
    for (option, value) in opts {
        let prefix = format!("Variance.{}", option.trim_end_matches('0'));
        let ids = names
            .iter()
            .enumerate()
            .filter(|(_, n)| n.starts_with(&prefix))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let values = coefficients(e, &value)?;
        if values.len() != ids.len() || values.iter().any(|x| !x.is_finite()) {
            return Err(err(
                "estimate",
                "InvalidInitialParameter",
                "Variance0 starts must be finite and match the corresponding active coefficient count.",
            ));
        }
        for (&i, &x) in ids.iter().zip(&values) {
            if original[i].is_nan() {
                p[i] = x;
            }
        }
    }
    Ok(())
}

pub(super) fn paths(e: &mut Engine, n: &str, model: &Model, a: &[Value]) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(err(n, "InvalidNumInputs", "Not enough input arguments."));
    }
    let count = integer(e, &a[0], 1, MAX_ELEMENTS)?;
    let forecast = n == "forecast";
    let positional = forecast && a.get(1).is_some_and(|v| text(v).is_err());
    let mut o = options(
        e,
        n,
        &a[if positional { 2 } else { 1 }..],
        if forecast {
            &["Y0", "E0", "V0", "X0", "XF"]
        } else {
            &["Y0", "E0", "V0", "X", "NumPaths"]
        },
    )?;
    if positional && o.insert("Y0".into(), a[1].clone()).is_some() {
        return Err(err(n, "DuplicateY0", "Specify Y0 only once."));
    }
    let mut m = model.clone();
    let xname = if forecast { "XF" } else { "X" };
    if !o.contains_key(xname) {
        if o.contains_key("X0") {
            return Err(err(n, "MissingXF", "X0 requires XF."));
        }
        m.beta.clear();
    }
    m.fully_specified()?;
    let vm = &m.conditional.as_ref().unwrap().model;
    // Explicit innovations make X0 irrelevant, as documented.
    if optional_matrix(e, &o, "E0")?.is_some() {
        o.remove("X0");
    }
    let mut pre = presamples(e, &o)?;
    let y0 = pre.remove(0);
    let mut e0 = pre.remove(0);
    let v0 = pre.remove(0);
    let x0 = pre.remove(0);
    if forecast && y0.is_none() {
        return Err(err(n, "MissingY0", "Presample responses Y0 are required."));
    }
    let paths = if forecast {
        [&y0, &e0, &v0]
            .into_iter()
            .flatten()
            .map(Matrix::cols)
            .max()
            .unwrap_or(1)
    } else {
        o.get("NumPaths")
            .map(|v| integer(e, v, 1, MAX_ELEMENTS))
            .transpose()?
            .unwrap_or(1)
    };
    let p = m.p();
    let q = m.q().max(vm.q());
    let vp = vm.variance_presample();
    check_pre(&y0, p, paths, "Y0", forecast)?;
    check_pre(&e0, q, paths, "E0", forecast)?;
    check_pre(&v0, vp, paths, "V0", forecast)?;
    preflight(e, count, paths, 8)?;
    let x = clean_aligned(e, vec![optional_matrix(e, &o, xname)?])?.remove(0);
    let x = if let Some(x) = x {
        if x.cols() != m.beta.len() || x.rows() < count {
            return Err(err(
                n,
                "PredictorSizeMismatch",
                "Predictors have too few rows or the wrong number of columns.",
            ));
        }
        Some(if forecast {
            Matrix::from_column_vec(
                count,
                x.cols(),
                (0..x.cols())
                    .flat_map(|j| x.data()[j * x.rows()..j * x.rows() + count].iter().copied())
                    .collect(),
            )?
        } else {
            tail_matrix(&x, count)?
        })
    } else {
        None
    };

    if forecast
        && e0.is_none()
        && let Some(y) = &y0
        && y.rows() > p
        && y.rows() >= p + q
    {
        // Infer using the first P responses as explicit Y0. Without X0,
        // the documentation drops the regression from this inference only.
        let mut mean = m.clone();
        if x0.is_none() {
            mean.beta.clear();
        }
        let mut opts = BTreeMap::new();
        if let Some(x0) = x0 {
            opts.insert("X".into(), Value::Matrix(x0));
        }
        let pre = Matrix::from_column_vec(
            p,
            y.cols(),
            (0..y.cols())
                .flat_map(|j| y.data()[j * y.rows()..j * y.rows() + p].iter().copied())
                .collect(),
        )?;
        opts.insert("Y0".into(), Value::Matrix(pre));
        let s = likelihood::sample(
            e,
            &mean,
            &Value::Matrix(tail_matrix(y, y.rows() - p)?),
            &opts,
        )?;
        preflight(e, s.y.rows(), paths, 4)?;
        let mut z = Vec::with_capacity(s.y.rows() * paths);
        for j in 0..paths {
            z.extend(likelihood::residuals(
                e,
                &mean,
                &s,
                if y.cols() == 1 { 0 } else { j },
            )?);
        }
        e0 = Some(Matrix::from_column_vec(s.y.rows(), paths, z)?);
        check_pre(&e0, q, paths, "E0 inferred from Y0", true)?;
    }
    let (a, b) = m.polynomials();
    let ar = sparse(&a);
    let ma = sparse(&b);
    let initial = if m.d == 0 && m.s == 0 && m.beta.is_empty() {
        m.c / a.iter().sum::<f64>()
    } else {
        0.
    };
    let mut ys = Vec::with_capacity(count * paths);
    let mut es = Vec::with_capacity(count * paths);
    let mut vs = Vec::with_capacity(count * paths);
    let mut mses = Vec::with_capacity(if forecast { count * paths } else { 0 });
    for j in 0..paths {
        e.check_cancel()?;
        let mut y = presample_column(&y0, j, p, initial);
        let mut z = presample_column(&e0, j, q, 0.);
        let mut v = if let Some(v0) = &v0 {
            full_column(v0, j).to_vec()
        } else if forecast
            && let Some(e0) = &e0
            && e0.rows() >= vm.p().max(vm.q()) + vm.p()
        {
            vol::conditional_variance(vm, full_column(e0, j), None, None, &e.cancel)?
        } else {
            vec![vm.unconditional_scale()?; vp]
        };
        let nv = v.len();
        let fv = if forecast {
            vol::forecast_path(vm, count, &z, &v, &e.cancel)?
        } else {
            Vec::new()
        };
        for t in 0..count {
            if t.is_multiple_of(1024) {
                e.check_cancel()?;
            }
            let h = if forecast {
                fv[t]
            } else {
                vol::one_step_forecast(vm, &z[z.len() - vm.q()..], &v[v.len() - vp..])?
            };
            let innovation = if forecast {
                0.
            } else {
                let normal = e.random.next_normal();
                let draw = if let Some(nu) = m.dof {
                    normal * ((nu - 2.) / (2. * forecast::gamma(e, nu / 2.)?)).sqrt()
                } else {
                    normal
                };
                draw * h.sqrt()
            };
            let mut response = m.c + innovation;
            for &(lag, x) in &ar {
                response -= x * y[p + t - lag];
            }
            for &(lag, x) in &ma {
                response += x * z[q + t - lag];
            }
            if let Some(x) = &x {
                for (k, beta) in m.beta.iter().enumerate() {
                    response += beta * x.data()[k * count + t];
                }
            }
            if !response.is_finite() {
                return Err(err(n, "NumericalFailure", "Response recursion overflowed."));
            }
            y.push(response);
            z.push(innovation);
            v.push(h);
        }
        ys.extend_from_slice(&y[p..]);
        es.extend_from_slice(&z[q..]);
        vs.extend_from_slice(&v[nv..]);
        if forecast {
            mses.extend(forecast_mse(e, &a, &b, &fv)?);
        }
    }
    Ok(vec![
        value_matrix(count, paths, ys)?,
        value_matrix(count, paths, if forecast { mses } else { es })?,
        value_matrix(count, paths, vs)?,
    ])
}

// A finite ARMA state [y_t,...,y_{t-p+1},e_t,...,e_{t-q+1}].
// P <- T P T' + V_t g g'; T is a companion row plus shifts. This computes
// sum_{i=0}^{h-1} psi_i^2 V_{h-i} in linear time in h, without FFT roundoff.
fn forecast_mse(e: &Engine, a: &[f64], b: &[f64], v: &[f64]) -> Result<Vec<f64>> {
    let p = (a.len() - 1).max(1);
    let q = b.len() - 1;
    let dim = p + q;
    preflight(e, dim, dim, 3)?;
    let mut weights = vec![0.; dim];
    for (i, &x) in a.iter().skip(1).enumerate() {
        weights[i] = -x;
    }
    weights[p..].copy_from_slice(&b[1..]);
    let mut cov = vec![0.; dim * dim];
    let mut next = cov.clone();
    let mut cross = vec![0.; dim];
    let mut result = Vec::with_capacity(v.len());
    for (t, &h) in v.iter().enumerate() {
        if t.is_multiple_of(256) {
            e.check_cancel()?;
        }
        for i in 0..dim {
            cross[i] = (0..dim).map(|j| cov[i * dim + j] * weights[j]).sum();
        }
        let mse = h + weights.iter().zip(&cross).map(|(w, c)| w * c).sum::<f64>();
        if !mse.is_finite() || mse <= 0. {
            return Err(err(
                "forecast",
                "NumericalFailure",
                "Forecast error variance recursion overflowed.",
            ));
        }
        result.push(mse);
        for i in 0..dim {
            for j in 0..dim {
                next[i * dim + j] = match (i, j) {
                    (0, 0) => mse,
                    (0, j) | (j, 0) => {
                        if j == p && q > 0 {
                            h
                        } else {
                            cross[j - 1]
                        }
                    }
                    (i, j) if q > 0 && (i == p || j == p) => {
                        if i == p && j == p {
                            h
                        } else {
                            0.
                        }
                    }
                    (i, j) => cov[(i - 1) * dim + j - 1],
                };
            }
        }
        std::mem::swap(&mut cov, &mut next);
    }
    Ok(result)
}
