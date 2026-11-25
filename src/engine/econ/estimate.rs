use super::*;
use nalgebra::{DMatrix, DVector};
const OPTIONS: &[&str] = &[
    "Y0",
    "E0",
    "V0",
    "X",
    "Display",
    "Constant0",
    "AR0",
    "SAR0",
    "MA0",
    "SMA0",
    "Beta0",
    "DoF0",
    "Variance0",
    "Options",
];
pub(super) fn estimate(
    e: &mut Engine,
    m: &Model,
    a: &[Value],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(err(
            "estimate",
            "InvalidNumInputs",
            "Not enough input arguments.",
        ));
    }
    let o = options(e, "estimate", &a[1..], OPTIONS)?;
    let mut template = m.clone();
    template.fit = None;
    if let Some(x) = likelihood::optional_matrix(e, &o, "X")? {
        if template.beta.is_empty() {
            if x.cols() > MAX_PARAMETERS - 2 {
                return Err(err("estimate", "SizeLimit", "Too many predictors."));
            }
            template.beta = vec![f64::NAN; x.cols()];
        }
    } else {
        template.beta.clear();
    }
    template.validate()?;
    let s = likelihood::sample(e, &template, &a[0], &o)?;
    let v0 = if template.conditional.is_some() {
        composite::estimation_presample(e, &template, &o)?
    } else {
        None
    };
    if s.y.cols() != 1 {
        return Err(err(
            "estimate",
            "InvalidData",
            "Estimation requires one response column.",
        ));
    }
    let original = template.parameters();
    let names = template.parameter_names();
    let free: Vec<_> = original
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_nan())
        .map(|(i, _)| i)
        .collect();
    if s.y.rows() <= free.len() || (s.y0.is_none() && s.y.rows() <= template.p().max(template.q()))
    {
        return Err(err(
            "estimate",
            "InsufficientData",
            "Too few observations for the requested model.",
        ));
    }
    preflight(e, s.y.rows(), free.len().max(1), 2)?;
    e.preflight_elements(
        s.y.rows().saturating_mul(free.len() + 8).saturating_add(
            original
                .len()
                .saturating_mul(original.len())
                .saturating_mul(3),
        ),
    )?;
    let mut settings = optimizer::Settings::default();
    let mut option_display = None;
    let mut algorithm = "sqp".to_string();
    if let Some(v) = o.get("Options") {
        if let Array::Struct(st) = v.clone().array(&e.cancel)? {
            if st.len() != 1 {
                return Err(err(
                    "estimate",
                    "InvalidOptions",
                    "Options must be a scalar optimset/optimoptions structure.",
                ));
            }
            for k in st.field_names() {
                let v = Value::from(st.get(st.field_index(k).unwrap(), 0)?.as_ref().clone());
                if v.clone().array(&e.cancel)?.is_empty() {
                    continue;
                }
                match k.to_ascii_lowercase().as_str() {
                    "maxiter" | "maxiterations" => {
                        settings.iterations = integer(e, &v, 0, 100_000)?
                    }
                    "maxfunevals" | "maxfunctionevaluations" => {
                        settings.evaluations = integer(e, &v, 1, 1_000_000)?
                    }
                    "tolfun" | "optimalitytolerance" => {
                        settings.gradient_tol = positive(e, &v, k)?;
                    }
                    "tolx" | "steptolerance" => settings.step_tol = positive(e, &v, k)?,
                    "functiontolerance" => settings.function_tol = positive(e, &v, k)?,
                    "constrainttolerance" => settings.constraint_tol = positive(e, &v, k)?,
                    "finitedifferencestepsize" => settings.difference_step = positive(e, &v, k)?,
                    "display" => {
                        let value = text(&v)?.to_ascii_lowercase();
                        if !matches!(value.as_str(), "off" | "none" | "iter" | "final") {
                            return Err(err(
                                "estimate",
                                "InvalidOptions",
                                "Display must be off, none, iter, or final.",
                            ));
                        }
                        option_display = Some(value);
                    }
                    "algorithm" => {
                        let name = text(&v)?;
                        if !matches!(
                            name.to_ascii_lowercase().as_str(),
                            "sqp" | "interior-point" | "active-set" | "quasi-newton"
                        ) {
                            return Err(err(
                                "estimate",
                                "UnsupportedOption",
                                "Algorithm must be sqp, interior-point, active-set, or quasi-newton (all use SQP).",
                            ));
                        }
                        algorithm = name.to_ascii_lowercase();
                    }
                    "finitedifferencetype" => {
                        let name = text(&v)?.to_ascii_lowercase();
                        if !matches!(name.as_str(), "central" | "forward") {
                            return Err(err(
                                "estimate",
                                "UnsupportedOption",
                                "FiniteDifferenceType must be central or forward.",
                            ));
                        }
                        settings.central = name == "central";
                    }
                    "diagnostics" => {
                        let value = text(&v)?;
                        if !matches!(value.as_str(), "on" | "off") {
                            return Err(err(
                                "estimate",
                                "InvalidOptions",
                                "Diagnostics must be on or off.",
                            ));
                        }
                    }
                    "solver" => {}
                    "specifyobjectivegradient" | "specifyconstraintgradient" | "checkgradients" => {
                        let a = v.clone().array(&e.cancel)?;
                        if !matches!(a, Array::Logical(ref b) if b.len() == 1 && b.data()[0] == 0)
                            && !matches!(a, Array::Numeric(ref x) if x.len() == 1 && x.data()[0] == 0.)
                        {
                            return Err(err(
                                "estimate",
                                "UnsupportedOption",
                                "User-supplied derivative options are not supported for built-in likelihoods.",
                            ));
                        }
                    }
                    _ => {
                        return Err(err(
                            "estimate",
                            "UnsupportedOption",
                            &format!("Options field '{k}' is not supported by estimate."),
                        ));
                    }
                }
            }
        } else if !v.clone().array(&e.cancel)?.is_empty() {
            return Err(err(
                "estimate",
                "InvalidOptions",
                "Options must be an optimset/optimoptions structure.",
            ));
        }
    }
    let explicit_display = o.get("Display").map(text).transpose()?;
    let display = explicit_display
        .as_deref()
        .unwrap_or("params")
        .to_ascii_lowercase();
    if !matches!(
        display.as_str(),
        "off" | "params" | "iter" | "diagnostics" | "full"
    ) {
        return Err(err(
            "estimate",
            "InvalidDisplay",
            "Display must be off, params, iter, diagnostics, or full.",
        ));
    }
    settings.iter_display = if display == "off" {
        false
    } else if explicit_display.is_some() {
        matches!(display.as_str(), "iter" | "full")
    } else {
        option_display.as_deref() == Some("iter")
    };
    let mut initial = if template.conditional.is_some() {
        composite::initial_parameters(e, &template, &s)?
    } else {
        initial_parameters_with_margin(e, &template, &s, settings.constraint_tol)?
    };
    for (opt, prefix) in [
        ("Constant0", "Constant"),
        ("AR0", "AR{"),
        ("SAR0", "SAR{"),
        ("MA0", "MA{"),
        ("SMA0", "SMA{"),
        ("Beta0", "Beta("),
        ("Variance0", "Variance"),
        ("DoF0", "DoF"),
    ] {
        if let Some(v) = o.get(opt) {
            if opt == "Variance0" && template.conditional.is_some() {
                composite::starting_variance(e, &template, v, &mut initial)?;
                continue;
            }
            let values = coefficients(e, v)?;
            let ids: Vec<_> = names
                .iter()
                .enumerate()
                .filter(|(_, s)| s.starts_with(prefix))
                .map(|(i, _)| i)
                .collect();
            if values.len() != ids.len() || values.iter().any(|x| !x.is_finite()) {
                return Err(err(
                    "estimate",
                    "InvalidInitialParameter",
                    &format!(
                        "{opt} must have one finite value for each active corresponding parameter."
                    ),
                ));
            }
            for (&i, &v) in ids.iter().zip(&values) {
                if original[i].is_nan() {
                    initial[i] = v;
                }
            }
        }
    }
    template.with_parameters(&initial).fully_specified()?;
    // Scaling changes coordinates only, preserving all starting values.
    let scale: Vec<_> = free
        .iter()
        .map(|&i| {
            if positive_variance(&template, &names[i]) {
                initial[i]
            } else if names[i] == "DoF" {
                10.
            } else {
                1.
            }
        })
        .collect();
    let x0: Vec<_> = free
        .iter()
        .zip(&scale)
        .map(|(&i, s)| initial[i] / s)
        .collect();
    let constraints = optimizer::constraints(
        &initial,
        &names,
        &free,
        &scale,
        template.conditional.as_ref().map(|v| v.model.kind),
        settings.constraint_tol,
    );
    let fit = optimizer::minimize(
        e,
        |engine, x| {
            let mut p = initial.clone();
            for ((&i, s), &v) in free.iter().zip(&scale).zip(x) {
                p[i] = v * s;
            }
            let model = template.with_parameters(&p);
            match contributions(engine, &model, &s, &v0) {
                Ok(ll) => Ok(-ll.iter().sum::<f64>() / s.y.rows() as f64),
                Err(_) if !engine.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    Ok(f64::INFINITY)
                }
                Err(err) => Err(err),
            }
        },
        x0,
        &constraints,
        &settings,
        out,
    )?;
    let mut params = initial.clone();
    for ((&i, s), &v) in free.iter().zip(&scale).zip(&fit.x) {
        params[i] = v * s;
    }
    let mut estimated = template.with_parameters(&params);
    estimated.fully_specified()?;
    let contributions = contributions(e, &estimated, &s, &v0)?;
    let logl = contributions.iter().sum();
    let covariance = covariance(e, &template, &params, &free, &s, &v0)?;
    estimated.fit = Some(Arc::new(arima::Fit {
        n: s.y.rows(),
        free: free.len(),
        logl,
        covariance: covariance.clone(),
        params: params.clone(),
        names,
    }));
    if let Some(v) = &mut estimated.conditional {
        v.object = super::super::objects::econ_vol_new(e, (*v.model).clone())?.array(&e.cancel)?;
    }
    let option_record = o
        .get("Options")
        .map(|v| v.clone().array(&e.cancel))
        .transpose()?
        .unwrap_or(record(
            e,
            vec![
                ("Algorithm", string(e, &algorithm)?),
                ("ConstraintTolerance", number(settings.constraint_tol)),
                ("OptimalityTolerance", number(settings.gradient_tol)),
                ("StepTolerance", number(settings.step_tol)),
                ("MaxIterations", number(settings.iterations as f64)),
                (
                    "MaxFunctionEvaluations",
                    number(settings.evaluations as f64),
                ),
            ],
        )?);
    let info = record(
        e,
        vec![
            ("exitflag", number(fit.flag)),
            ("options", option_record),
            ("X", column(params.clone())?),
            ("X0", column(initial)?),
        ],
    )?;
    if matches!(display.as_str(), "params" | "full") {
        e.emit(out, display::summary_text(&estimated))?;
    }
    if matches!(display.as_str(), "diagnostics" | "full") {
        e.emit(out, format!("Conditional maximum likelihood, SQP: exit flag {}, {} estimated parameters, {} effective observations.", fit.flag, free.len(), s.y.rows()))?;
    }
    Ok(vec![
        super::super::objects::arima_new(e, estimated)?,
        value_matrix(params.len(), params.len(), covariance)?,
        Value::Matrix(Matrix::scalar(logl)),
        Value::from(info),
    ])
}
fn positive(e: &Engine, v: &Value, k: &str) -> Result<f64> {
    let x = scalar(e, v)?;
    if !x.is_finite() || x <= 0. {
        Err(err(
            "estimate",
            "InvalidOptions",
            &format!("{k} must be finite and positive."),
        ))
    } else {
        Ok(x)
    }
}
pub(super) fn initial_parameters(
    e: &Engine,
    m: &Model,
    s: &likelihood::Sample,
) -> Result<Vec<f64>> {
    initial_parameters_with_margin(e, m, s, 1e-7)
}

fn initial_parameters_with_margin(
    e: &Engine,
    m: &Model,
    s: &likelihood::Sample,
    constraint_tol: f64,
) -> Result<Vec<f64>> {
    let y = s.y.data();
    let n = y.len();
    let mut initial = m.clone();
    for v in &mut initial.beta {
        if v.is_nan() {
            *v = 0.;
        }
    }
    // Estimate Beta (and Constant if undifferenced) by OLS when X is present.
    let mut u = y.to_vec();
    if let Some(xm) = &s.x {
        let free_beta: Vec<_> = m
            .beta
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_nan())
            .map(|(i, _)| i)
            .collect();
        let cfree = usize::from(m.d == 0 && m.s == 0 && m.c.is_nan());
        let k = free_beta.len() + cfree;
        if k > 0 && n > k {
            let mut x = DMatrix::zeros(n, k);
            let mut target = DVector::from_column_slice(y);
            for t in 0..n {
                if t % 4096 == 0 {
                    e.check_cancel()?;
                }
                if cfree > 0 {
                    x[(t, 0)] = 1.;
                } else if m.d == 0 && m.s == 0 && m.c.is_finite() {
                    target[t] -= m.c;
                }
                for (j, &col) in free_beta.iter().enumerate() {
                    x[(t, j + cfree)] = xm.data()[col * n + t];
                }
                for (col, &b) in m.beta.iter().enumerate() {
                    if !b.is_nan() {
                        target[t] -= b * xm.data()[col * n + t];
                    }
                }
            }
            if let Ok(sol) = x.clone().svd(true, true).solve(&target, 1e-12) {
                if cfree > 0 && sol[0].is_finite() {
                    initial.c = sol[0];
                }
                for (j, &col) in free_beta.iter().enumerate() {
                    if sol[j + cfree].is_finite() {
                        initial.beta[col] = sol[j + cfree];
                    }
                }
            }
        }
        for (t, u_t) in u.iter_mut().enumerate().take(n) {
            let mut xb = 0.;
            for (col, &b) in initial.beta.iter().enumerate() {
                xb += b * xm.data()[col * n + t];
            }
            *u_t -= xb;
        }
    }
    let mut differenced = vec![];
    // Build a finite differencing-only filter; no unknown coefficients enter.
    let diff = Model {
        d: m.d,
        s: m.s,
        ..Model::default()
    };
    let (d, _) = diff.polynomials();
    for t in d.len() - 1..u.len() {
        let mut v = u[t];
        for (lag, c) in d.iter().enumerate().skip(1) {
            v += c * u[t - lag];
        }
        differenced.push(v);
    }
    let mean = differenced.iter().sum::<f64>() / differenced.len().max(1) as f64;
    let variance = (differenced.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
        / differenced.len().max(1) as f64)
        .max(1e-8);
    if initial.c.is_nan() {
        initial.c = mean;
    }
    for coeff in [
        &mut initial.ar,
        &mut initial.sar,
        &mut initial.ma,
        &mut initial.sma,
    ] {
        for v in coeff {
            if v.is_nan() {
                *v = 0.;
            }
        }
    }
    if initial.variance.is_nan() {
        initial.variance = variance;
    }
    if initial.dof.is_some_and(|v| v.is_nan()) {
        initial.dof = Some(10.);
    }
    let has_ma = m.ma.iter().any(|&v| v != 0.) || m.sma.iter().any(|&v| v != 0.);
    // OLS start for a nonseasonal pure AR, preserving fixed coefficients. This
    // also matches AutoReg conditional fits when explicit Y0 is supplied.
    if m.d == 0 && m.s == 0 && m.sar.is_empty() && !has_ma && s.x.is_none() {
        let active: Vec<_> =
            m.ar.iter()
                .enumerate()
                .filter(|(_, v)| v.is_nan())
                .map(|(i, _)| i + 1)
                .collect();
        let cfree = usize::from(m.c.is_nan());
        let k = active.len() + cfree;
        let p = m.p();
        let pre = if s.y0.is_some() {
            likelihood::presample_column(&s.y0, 0, p, mean)
        } else {
            vec![mean; p]
        };
        let mut hist = pre;
        hist.extend_from_slice(y);
        if k > 0 {
            let mut x = DMatrix::zeros(n, k);
            let mut target = DVector::from_column_slice(y);
            for t in 0..n {
                e.check_cancel()?;
                if cfree > 0 {
                    x[(t, 0)] = 1.;
                } else {
                    target[t] -= m.c;
                }
                for (j, &lag) in active.iter().enumerate() {
                    x[(t, j + cfree)] = hist[p + t - lag];
                }
                for (j, v) in m.ar.iter().enumerate() {
                    if !v.is_nan() {
                        target[t] -= v * hist[p + t - j - 1];
                    }
                }
            }
            if let Ok(solution) = x.clone().svd(true, true).solve(&target, 1e-12) {
                if cfree > 0 {
                    initial.c = solution[0];
                }
                for (j, &lag) in active.iter().enumerate() {
                    initial.ar[lag - 1] = solution[j + cfree];
                }
                let resid = &target - &x * &solution;
                if m.variance.is_nan() {
                    initial.variance = (resid.dot(&resid) / n.max(1) as f64).max(1e-8);
                }
            }
        }
    } else {
        // Hannan-Rissanen (long-AR residual regression) when MA/SMA are present,
        // or OLS on the differenced series for general AR/SAR/ARIMAX models.
        hannan_rissanen_start(e, m, &differenced, mean, has_ma, &mut initial)?;
    }
    let margin = (constraint_tol * 2.).clamp(1e-5, 0.5);
    shrink_polynomial(&mut initial.ar, &m.ar, -1., margin);
    shrink_polynomial(&mut initial.sar, &m.sar, -1., margin);
    shrink_polynomial(&mut initial.ma, &m.ma, 1., margin);
    shrink_polynomial(&mut initial.sma, &m.sma, 1., margin);
    Ok(initial.parameters_with_template(m))
}

fn free_lags(template: &[f64]) -> Vec<usize> {
    template
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_nan())
        .map(|(i, _)| i + 1)
        .collect()
}

fn hannan_rissanen_start(
    e: &Engine,
    m: &Model,
    w: &[f64],
    mean: f64,
    has_ma: bool,
    initial: &mut Model,
) -> Result<()> {
    let wn = w.len();
    let ar_free = free_lags(&m.ar);
    let sar_free = free_lags(&m.sar);
    let ma_free = free_lags(&m.ma);
    let sma_free = free_lags(&m.sma);
    let cfree = usize::from(m.c.is_nan());
    let k = cfree + ar_free.len() + sar_free.len() + ma_free.len() + sma_free.len();
    if k == 0 {
        return Ok(());
    }
    let max_lag = m.ar.len().max(m.sar.len()).max(m.ma.len()).max(m.sma.len());
    let mut e_hat = vec![0.; wn];
    let mut start = max_lag;
    if has_ma {
        let l_ord = ((wn as f64).powf(0.35).ceil() as usize)
            .max(max_lag + 2)
            .min(wn / 3)
            .min(24);
        if l_ord >= 1 && wn > l_ord + 4 {
            let rows = wn - l_ord;
            let mut x_long = DMatrix::zeros(rows, l_ord);
            let mut y_long = DVector::zeros(rows);
            for t in 0..rows {
                if t % 4096 == 0 {
                    e.check_cancel()?;
                }
                y_long[t] = w[l_ord + t] - mean;
                for lag in 1..=l_ord {
                    x_long[(t, lag - 1)] = w[l_ord + t - lag] - mean;
                }
            }
            if let Ok(phi) = x_long.clone().svd(true, true).solve(&y_long, 1e-12) {
                let resid = &y_long - &x_long * &phi;
                for t in 0..rows {
                    if resid[t].is_finite() {
                        e_hat[l_ord + t] = resid[t];
                    }
                }
                start = start.max(l_ord);
            }
        }
    }
    if wn <= start + k + 2 {
        return Ok(());
    }
    let rows = wn - start;
    let mut x = DMatrix::zeros(rows, k);
    let mut target = DVector::zeros(rows);
    for t in 0..rows {
        if t % 4096 == 0 {
            e.check_cancel()?;
        }
        let idx = start + t;
        let mut rhs_fixed = 0.;
        if cfree > 0 {
            x[(t, 0)] = 1.;
        } else {
            rhs_fixed += m.c;
        }
        let mut col = cfree;
        for &lag in &ar_free {
            x[(t, col)] = w[idx - lag];
            col += 1;
        }
        for &lag in &sar_free {
            x[(t, col)] = w[idx - lag];
            col += 1;
        }
        for &lag in &ma_free {
            x[(t, col)] = e_hat[idx - lag];
            col += 1;
        }
        for &lag in &sma_free {
            x[(t, col)] = e_hat[idx - lag];
            col += 1;
        }
        for (j, &v) in m.ar.iter().chain(&m.sar).enumerate() {
            let lag = if j < m.ar.len() {
                j + 1
            } else {
                j - m.ar.len() + 1
            };
            if v.is_finite() && v != 0. {
                rhs_fixed += v * w[idx - lag];
            }
        }
        for (j, &v) in m.ma.iter().chain(&m.sma).enumerate() {
            let lag = if j < m.ma.len() {
                j + 1
            } else {
                j - m.ma.len() + 1
            };
            if v.is_finite() && v != 0. {
                rhs_fixed += v * e_hat[idx - lag];
            }
        }
        target[t] = w[idx] - rhs_fixed;
    }
    if let Ok(sol) = x.clone().svd(true, true).solve(&target, 1e-12)
        && sol.iter().all(|v| v.is_finite())
    {
        if cfree > 0 {
            initial.c = sol[0];
        }
        let mut col = cfree;
        for &lag in &ar_free {
            initial.ar[lag - 1] = sol[col];
            col += 1;
        }
        for &lag in &sar_free {
            initial.sar[lag - 1] = sol[col];
            col += 1;
        }
        for &lag in &ma_free {
            initial.ma[lag - 1] = sol[col];
            col += 1;
        }
        for &lag in &sma_free {
            initial.sma[lag - 1] = sol[col];
            col += 1;
        }
        let resid = &target - &x * &sol;
        if m.variance.is_nan() {
            initial.variance = (resid.dot(&resid) / rows.max(1) as f64).max(1e-8);
        }
    }
    Ok(())
}

fn stable_with_margin(c: &[f64], sign: f64, margin: f64) -> bool {
    let mut a: Vec<_> = std::iter::once(1.)
        .chain(
            c.iter()
                .enumerate()
                .map(|(j, x)| (sign * x) / (1. - margin).powi((j + 1) as i32)),
        )
        .collect();
    for k in (1..a.len()).rev() {
        let r = a[k];
        if !r.is_finite() || r.abs() >= 1. - 2e-12 {
            return false;
        }
        let den = 1. - r * r;
        let mut next = vec![1.; k];
        for i in 1..k {
            next[i] = (a[i] - r * a[k - i]) / den;
        }
        a = next;
    }
    true
}

fn shrink_polynomial(coeff: &mut [f64], template: &[f64], sign: f64, margin: f64) {
    if stable_with_margin(coeff, sign, margin) {
        return;
    }
    for _ in 0..80 {
        for (v, orig) in coeff.iter_mut().zip(template) {
            if orig.is_nan() {
                *v *= 0.8;
            }
        }
        if stable_with_margin(coeff, sign, margin) {
            return;
        }
    }
    for (v, orig) in coeff.iter_mut().zip(template) {
        if orig.is_nan() {
            *v = 0.;
        }
    }
}
impl Model {
    // Do not infer active slots from zero-valued initial coefficients.
    fn parameters_with_template(&self, template: &Self) -> Vec<f64> {
        let mut p = vec![self.c];
        for (a, t) in [
            (&self.ar, &template.ar),
            (&self.sar, &template.sar),
            (&self.ma, &template.ma),
            (&self.sma, &template.sma),
        ] {
            for (&x, &mask) in a.iter().zip(t) {
                if mask != 0. {
                    p.push(x);
                }
            }
        }
        p.extend(&self.beta);
        p.push(self.variance);
        if let Some(d) = self.dof {
            p.push(d);
        }
        p
    }
}
fn covariance(
    e: &Engine,
    m: &Model,
    p: &[f64],
    free: &[usize],
    s: &likelihood::Sample,
    v0: &Option<Matrix>,
) -> Result<Vec<f64>> {
    let n = s.y.rows();
    let k = free.len();
    let total = p.len();
    preflight(e, n, k.max(1), 2)?;
    let mut score = DMatrix::zeros(n, k);
    let names = m.parameter_names();
    for (j, &i) in free.iter().enumerate() {
        let mut h = 6e-6 * (1. + p[i].abs());
        if positive_variance(m, &names[i]) {
            h = h.min(p[i] * 1e-4);
        } else if names[i] == "DoF" {
            h = h.min((p[i] - 2.) * 1e-4);
        }
        if h == 0. {
            return Err(err(
                "estimate",
                "CovarianceFailure",
                "A parameter is too close to its bound for numerical covariance estimation.",
            ));
        }
        let (la, lb, denom) = loop {
            e.check_cancel()?;
            let mut plus = p.to_vec();
            let mut minus = p.to_vec();
            plus[i] += h;
            minus[i] -= h;
            let a = contributions(e, &m.with_parameters(&plus), s, v0);
            let b = contributions(e, &m.with_parameters(&minus), s, v0);
            if let (Ok(a), Ok(b)) = (&a, &b) {
                break (a.clone(), b.clone(), 2. * h);
            }
            if h <= 1e-10 * (1. + p[i].abs()) {
                let base = contributions(e, &m.with_parameters(p), s, v0)?;
                if let Ok(a) = a {
                    break (a, base, h);
                }
                if let Ok(b) = b {
                    break (base, b, h);
                }
                return Err(err(
                    "estimate",
                    "CovarianceFailure",
                    "Could not evaluate likelihood scores within the parameter bounds.",
                ));
            }
            h *= 0.1;
        };
        for t in 0..n {
            score[(t, j)] = (la[t] - lb[t]) / denom;
        }
    }
    if score.iter().any(|v| !v.is_finite()) {
        return Err(err(
            "estimate",
            "CovarianceFailure",
            "Nonfinite likelihood scores prevent covariance estimation.",
        ));
    }
    let mut covariance = vec![0.; total * total];
    if k > 0 {
        let inv = super::super::econ_vol::opg_covariance(&score, &e.cancel)?;
        for (j, &col) in free.iter().enumerate() {
            for (i, &row) in free.iter().enumerate() {
                covariance[col * total + row] = inv[(i, j)];
            }
        }
    }
    Ok(covariance)
}

fn contributions(
    e: &Engine,
    m: &Model,
    s: &likelihood::Sample,
    v0: &Option<Matrix>,
) -> Result<Vec<f64>> {
    let ll = if m.conditional.is_some() {
        composite::estimation_contributions(e, m, s, v0)?
    } else {
        likelihood::contributions(m, &likelihood::residuals(e, m, s, 0)?)
    };
    if ll.iter().all(|v| v.is_finite()) {
        Ok(ll)
    } else {
        Err(err(
            "estimate",
            "UndefinedLikelihood",
            "Likelihood is undefined at these parameters.",
        ))
    }
}

fn positive_variance(m: &Model, name: &str) -> bool {
    name == "Variance"
        || (name == "Variance.Constant"
            && m.conditional
                .as_ref()
                .is_some_and(|v| v.model.kind != super::super::econ_vol::Kind::Egarch))
}
