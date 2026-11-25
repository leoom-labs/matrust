use super::*;
use crate::engine::econ::optimizer;
use nalgebra::DMatrix;
#[derive(Clone, Copy)]
enum Parameter {
    Constant,
    Garch(usize),
    Arch(usize),
    Leverage(usize),
    DoF,
    Offset,
}
fn parameters(m: &Model) -> Vec<Parameter> {
    let mut p = vec![Parameter::Constant];
    for (which, c) in [(0, &m.garch), (1, &m.arch), (2, &m.leverage)] {
        for (j, &x) in c.iter().enumerate() {
            if x != 0. {
                p.push(match which {
                    0 => Parameter::Garch(j),
                    1 => Parameter::Arch(j),
                    _ => Parameter::Leverage(j),
                });
            }
        }
    }
    if m.dof.is_some() {
        p.push(Parameter::DoF);
    }
    if m.offset != 0. {
        p.push(Parameter::Offset);
    }
    p
}
fn get(m: &Model, p: Parameter) -> f64 {
    match p {
        Parameter::Constant => m.constant,
        Parameter::Garch(j) => m.garch[j],
        Parameter::Arch(j) => m.arch[j],
        Parameter::Leverage(j) => m.leverage[j],
        Parameter::DoF => m.dof.unwrap(),
        Parameter::Offset => m.offset,
    }
}
pub(in crate::engine) fn initial_values(template: &Model, initial: &Model) -> Vec<f64> {
    parameters(template)
        .into_iter()
        .map(|p| get(initial, p))
        .collect()
}
fn put(m: &mut Model, p: Parameter, x: f64) {
    match p {
        Parameter::Constant => m.constant = x,
        Parameter::Garch(j) => m.garch[j] = x,
        Parameter::Arch(j) => m.arch[j] = x,
        Parameter::Leverage(j) => m.leverage[j] = x,
        Parameter::DoF => m.dof = Some(x),
        Parameter::Offset => m.offset = x,
    }
}
fn contributions(
    m: &Model,
    y: &[f64],
    ep: Option<&[f64]>,
    vp: Option<&[f64]>,
    c: &AtomicBool,
) -> Result<Vec<f64>> {
    let inn = y.iter().map(|x| x - m.offset).collect::<Vec<_>>();
    let v = recursion::estimation_variance(m, &inn, ep, vp, c)?;
    recursion::likelihood(m, &inn, &v, c)
}
pub(super) fn call(
    e: &mut Engine,
    template: &Model,
    a: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err("Not enough input arguments.".into());
    }
    let y = recursion::clean_data(e, &matrix(e, &a[0])?)?;
    if y.cols() != 1 || y.rows() == 0 {
        return Err(err(
            "estimate",
            "InvalidData",
            "y must be a nonempty numeric column vector.",
        ));
    }
    let opts = pairs(
        &a[1..],
        &[
            "E0",
            "V0",
            "Display",
            "Constant0",
            "GARCH0",
            "ARCH0",
            "Leverage0",
            "Offset0",
            "DoF0",
            "Options",
        ],
    )?;
    if template.kind == Kind::Garch && opts.contains_key("leverage0") {
        return Err(err(
            "estimate",
            "InvalidNameValue",
            "GARCH models have no leverage parameters.",
        ));
    }
    let (ep, vp) = recursion::presamples(e, &opts)?;
    for p in [ep.as_ref(), vp.as_ref()].into_iter().flatten() {
        if p.cols() != 1 {
            return Err(err(
                "estimate",
                "InvalidPresample",
                "Estimation presamples must be column vectors.",
            ));
        }
    }
    let (ep, vp) = (ep.as_ref().map(Matrix::data), vp.as_ref().map(Matrix::data));
    if ep.is_some_and(|p| p.len() < template.q())
        || vp.is_some_and(|p| p.len() < template.variance_presample())
    {
        return Err(err(
            "estimate",
            "InsufficientPresample",
            "Insufficient presample observations for model degrees.",
        ));
    }
    if vp.is_some_and(|p| p.iter().any(|x| *x <= 0.)) {
        return Err(err(
            "estimate",
            "InvalidPresample",
            "Presample variances must be positive.",
        ));
    }
    let par = parameters(template);
    let free = par
        .iter()
        .copied()
        .filter(|p| get(template, *p).is_nan())
        .collect::<Vec<_>>();
    if free.len() > 128 {
        return Err(err(
            "estimate",
            "TooManyParameters",
            "At most 128 estimated parameters are supported.",
        ));
    }
    e.preflight_elements(y.len().saturating_mul(free.len() + 8) + par.len() * par.len() * 3)?;
    let mean = y.data().iter().sum::<f64>() / y.len() as f64;
    let offset = if template.offset.is_nan() {
        mean
    } else {
        template.offset
    };
    let variance = y.data().iter().map(|x| (x - offset).powi(2)).sum::<f64>() / y.len() as f64;
    if !variance.is_finite() || variance <= 0. {
        return Err(err(
            "estimate",
            "InvalidData",
            "Estimation requires a positive finite sample second moment.",
        ));
    }
    let mut initial = initial_model(template, variance, mean);
    for (name, group) in [
        ("constant0", 0),
        ("garch0", 1),
        ("arch0", 2),
        ("leverage0", 3),
        ("dof0", 4),
        ("offset0", 5),
    ] {
        if let Some(v) = opts.get(name) {
            let val = matrix(e, v)?;
            if val.rows() > 1 && val.cols() > 1 {
                return Err(err(
                    "estimate",
                    "InvalidStart",
                    "Starting values must be numeric vectors.",
                ));
            }
            let selected = par
                .iter()
                .copied()
                .filter(|p| {
                    matches!(
                        (group, p),
                        (0, Parameter::Constant)
                            | (1, Parameter::Garch(_))
                            | (2, Parameter::Arch(_))
                            | (3, Parameter::Leverage(_))
                            | (4, Parameter::DoF)
                            | (5, Parameter::Offset)
                    )
                })
                .collect::<Vec<_>>();
            if val.len() != selected.len() || val.data().iter().any(|x| !x.is_finite()) {
                return Err(err(
                    "estimate",
                    "InvalidStart",
                    "Starting values must be finite and match the corresponding coefficient count.",
                ));
            }
            for (p, &x) in selected.iter().zip(val.data()) {
                if (matches!(p, Parameter::Constant) && template.kind != Kind::Egarch && x <= 0.)
                    || (matches!(p, Parameter::Garch(_) | Parameter::Arch(_))
                        && template.kind != Kind::Egarch
                        && x < 0.)
                    || (matches!(p, Parameter::DoF) && x <= 2.)
                {
                    return Err(err(
                        "estimate",
                        "InvalidStart",
                        "Starting variance parameters must be nonnegative, Constant a positive scalar, and DoF greater than 2.",
                    ));
                }
                if get(template, *p).is_nan() {
                    put(&mut initial, *p, x);
                }
            }
        }
    }
    initial.complete()?;
    let scale = free
        .iter()
        .map(|p| match p {
            Parameter::Constant if template.kind != Kind::Egarch => variance,
            Parameter::Offset => variance.sqrt(),
            Parameter::DoF => 10.,
            _ => 1.,
        })
        .collect::<Vec<_>>();
    let x0 = free
        .iter()
        .zip(&scale)
        .map(|(p, s)| get(&initial, *p) / s)
        .collect::<Vec<_>>();
    let mut options = optimizer::Settings::default();
    let mut algorithm = "sqp".to_string();
    let mut option_display = None;
    let mut display = vec!["params".to_string()];
    if let Some(v) = opts.get("options") {
        let a = (*v).clone().array(&e.cancel)?;
        let Array::Struct(s) = a else {
            return Err(err(
                "estimate",
                "InvalidOptions",
                "Options must be an optimoptions scalar structure.",
            ));
        };
        if s.len() != 1 {
            return Err(err("estimate", "InvalidOptions", "Options must be scalar."));
        }
        for (i, n) in s.field_names().iter().enumerate() {
            let v = Value::from(s.get(i, 0)?.as_ref().clone());
            if v.clone().array(&e.cancel)?.is_empty() {
                continue;
            }
            match n.as_str() {
                "MaxIterations" | "MaxIter" => options.iterations = order(e, &v, n, true, 100000)?,
                "MaxFunctionEvaluations" | "MaxFunEvals" => {
                    options.evaluations = order(e, &v, n, false, 1000000)?
                }
                "OptimalityTolerance" | "TolFun" => options.gradient_tol = positive(e, &v, n)?,
                "StepTolerance" | "TolX" => options.step_tol = positive(e, &v, n)?,
                "Display" => {
                    let t = text(&v)?.to_lowercase();
                    if !matches!(t.as_str(), "off" | "none" | "iter" | "final") {
                        return Err(err(
                            "estimate",
                            "InvalidOptions",
                            "Display must be off, none, iter, or final.",
                        ));
                    }
                    option_display = Some(t);
                }
                "Algorithm" => {
                    let t = text(&v)?.to_ascii_lowercase();
                    if !matches!(
                        t.as_str(),
                        "sqp" | "interior-point" | "active-set" | "quasi-newton"
                    ) {
                        return Err(err(
                            "estimate",
                            "InvalidOptions",
                            "Unsupported optimization Algorithm.",
                        ));
                    }
                    algorithm = t;
                }
                "FunctionTolerance" => options.function_tol = positive(e, &v, n)?,
                "FiniteDifferenceStepSize" => options.difference_step = positive(e, &v, n)?,
                "FiniteDifferenceType" => {
                    let t = text(&v)?;
                    options.central = match t.as_str() {
                        "central" => true,
                        "forward" => false,
                        _ => {
                            return Err(err(
                                "estimate",
                                "InvalidOptions",
                                "FiniteDifferenceType must be central or forward.",
                            ));
                        }
                    };
                }
                "ConstraintTolerance" => {
                    options.constraint_tol = positive(e, &v, n)?;
                }
                // Options produced by optimoptions contain solver metadata and defaults.
                "Solver" | "Diagnostics" => {}
                "SpecifyObjectiveGradient" | "SpecifyConstraintGradient" | "CheckGradients" => {
                    let a = v.clone().array(&e.cancel)?;
                    if !matches!(a,Array::Logical(ref b) if b.len()==1 && b.data()[0]==0)
                        && !matches!(a,Array::Numeric(ref x) if x.len()==1 && x.data()[0]==0.)
                    {
                        return Err(err(
                            "estimate",
                            "InvalidOptions",
                            "User-supplied derivative options are not supported for built-in likelihoods.",
                        ));
                    }
                }
                _ => {
                    return Err(err(
                        "estimate",
                        "InvalidOptions",
                        &format!("Unsupported optimization option '{n}'."),
                    ));
                }
            }
        }
    }
    let explicit_display = opts.contains_key("display");
    if let Some(v) = opts.get("display") {
        display = if let Ok(t) = text(v) {
            vec![t.to_lowercase()]
        } else {
            let a = (*v).clone().array(&e.cancel)?;
            match a {
                Array::Cell(c) => (0..c.len())
                    .map(|i| {
                        text(&Value::from(c.get_linear(i)?.as_ref().clone()))
                            .map(|t| t.to_lowercase())
                    })
                    .collect::<Result<Vec<_>>>()?,
                Array::Str(s) => s
                    .data()
                    .iter()
                    .map(|x| {
                        x.as_ref()
                            .map(|x| x.to_lowercase())
                            .ok_or("Missing display option".into())
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => {
                    return Err(err(
                        "estimate",
                        "InvalidDisplay",
                        "Display must be scalar text or a text vector.",
                    ));
                }
            }
        };
    }
    if display.iter().any(|s| {
        !matches!(
            s.as_str(),
            "off" | "params" | "diagnostics" | "full" | "iter"
        )
    }) {
        return Err(err("estimate", "InvalidDisplay", "Invalid Display option."));
    }
    let c = e.cancel.clone();
    options.iter_display = if display.iter().any(|s| s == "off") {
        false
    } else if explicit_display {
        display
            .iter()
            .any(|s| matches!(s.as_str(), "iter" | "full"))
    } else {
        option_display.as_deref() == Some("iter")
    };
    let build = |x: &[f64]| {
        let mut m = initial.clone();
        for ((p, s), &x) in free.iter().zip(&scale).zip(x) {
            put(&mut m, *p, x * s);
        }
        m
    };
    let initial_values: Vec<_> = par.iter().map(|p| get(&initial, *p)).collect();
    let free_indices: Vec<_> = par
        .iter()
        .enumerate()
        .filter(|(_, p)| get(template, **p).is_nan())
        .map(|(i, _)| i)
        .collect();
    let constraints = optimizer::constraints(
        &initial_values,
        &template.parameter_names(),
        &free_indices,
        &scale,
        Some(template.kind),
        options.constraint_tol,
    );
    let solution = optimizer::minimize(
        e,
        |_, x| {
            cancel(&c)?;
            let m = build(x);
            match contributions(&m, y.data(), ep, vp, &c) {
                Ok(v) => Ok(-v.iter().sum::<f64>() / y.len() as f64),
                Err(err) if c.load(Ordering::Relaxed) => Err(err),
                Err(_) => Ok(f64::INFINITY),
            }
        },
        x0,
        &constraints,
        &options,
        out,
    )?;
    let mut fitted = build(&solution.x);
    fitted.complete()?;
    let likelihood = contributions(&fitted, y.data(), ep, vp, &c)?;
    let logl = likelihood.iter().sum::<f64>();
    // MathWorks estimate, EstParamCov: outer product of per-observation scores,
    // not inverse observed Hessian, no sandwich or degrees-of-freedom correction.
    let k = free.len();
    let mut scores = DMatrix::zeros(y.len(), k);
    for (j, p) in free.iter().enumerate() {
        cancel(&c)?;
        let x = get(&fitted, *p);
        let mut h = 1e-5 * scale[j].max(x.abs());
        let (hi, lo, denom) = loop {
            let mut hi = fitted.clone();
            put(&mut hi, *p, x + h);
            let mut lo = fitted.clone();
            put(&mut lo, *p, x - h);
            let a = contributions(&hi, y.data(), ep, vp, &c);
            let b = contributions(&lo, y.data(), ep, vp, &c);
            if let (Ok(a), Ok(b)) = (&a, &b) {
                break (a.clone(), b.clone(), 2. * h);
            }
            if h < 1e-10 * scale[j] {
                if let Ok(a) = a {
                    break (a, likelihood.clone(), h);
                }
                if let Ok(b) = b {
                    break (likelihood.clone(), b, h);
                }
                return Err(err(
                    "estimate",
                    "InvalidCovariance",
                    "Could not evaluate likelihood scores.",
                ));
            }
            h *= 0.1;
        };
        for i in 0..y.len() {
            scores[(i, j)] = (hi[i] - lo[i]) / denom;
        }
    }
    let covfree = opg_covariance(&scores, &c)?;
    let mut covariance = vec![0.; par.len() * par.len()];
    let indices = par
        .iter()
        .enumerate()
        .filter(|(_, p)| get(template, **p).is_nan())
        .map(|(j, _)| j)
        .collect::<Vec<_>>();
    for (j, &col) in indices.iter().enumerate() {
        for (i, &row) in indices.iter().enumerate() {
            covariance[col * par.len() + row] = covfree[(i, j)];
        }
    }
    let covariance = Matrix::from_column_vec(par.len(), par.len(), covariance)?;
    fitted.fit = Some(Arc::new(model::Fit {
        values: par.iter().map(|p| get(&fitted, *p)).collect(),
        names: template.parameter_names(),
        covariance: covariance.clone(),
        sample_size: y.len(),
        free: k,
        log_likelihood: logl,
    }));
    if display
        .iter()
        .any(|s| matches!(s.as_str(), "params" | "full"))
    {
        e.emit(out, display::summary_body(&fitted)?)?;
    }
    if display
        .iter()
        .any(|s| matches!(s.as_str(), "diagnostics" | "full"))
    {
        e.emit(
            out,
            format!(
                "estimate: exitflag {}, iterations {}, function evaluations {}",
                solution.flag, solution.iterations, solution.evaluations
            ),
        )?;
    }
    let x = par.iter().map(|p| get(&fitted, *p)).collect::<Vec<_>>();
    let initial_values = par.iter().map(|p| get(&initial, *p)).collect::<Vec<_>>();
    let opt_struct = structure(vec![
        ("Algorithm", string_array(&algorithm)?),
        (
            "MaxFunctionEvaluations",
            Array::Numeric(Matrix::scalar(options.evaluations as f64)),
        ),
        (
            "StepTolerance",
            Array::Numeric(Matrix::scalar(options.step_tol)),
        ),
        (
            "FunctionTolerance",
            Array::Numeric(Matrix::scalar(options.function_tol)),
        ),
        (
            "ConstraintTolerance",
            Array::Numeric(Matrix::scalar(options.constraint_tol)),
        ),
        (
            "FiniteDifferenceType",
            string_array(if options.central {
                "central"
            } else {
                "forward"
            })?,
        ),
        (
            "FiniteDifferenceStepSize",
            Array::Numeric(Matrix::scalar(options.difference_step)),
        ),
        ("Display", string_array(&display.join(" "))?),
        (
            "MaxIterations",
            Array::Numeric(Matrix::scalar(options.iterations as f64)),
        ),
        (
            "OptimalityTolerance",
            Array::Numeric(Matrix::scalar(options.gradient_tol)),
        ),
    ])?
    .array(&c)?;
    let info = structure(vec![
        ("exitflag", Array::Numeric(Matrix::scalar(solution.flag))),
        ("options", opt_struct),
        ("X", Array::Numeric(Matrix::from_column_vec(x.len(), 1, x)?)),
        (
            "X0",
            Array::Numeric(Matrix::from_column_vec(
                initial_values.len(),
                1,
                initial_values,
            )?),
        ),
    ])?;
    let object = super::super::objects::econ_vol_new(e, fitted)?;
    Ok(vec![object, Value::Matrix(covariance), number(logl), info]
        .into_iter()
        .take(nout.max(1))
        .collect())
}
fn positive(e: &Engine, v: &Value, n: &str) -> Result<f64> {
    let x = scalar(e, v)?;
    if x.is_finite() && x > 0. {
        Ok(x)
    } else {
        Err(err(
            "estimate",
            "InvalidOptions",
            &format!("{n} must be finite and positive."),
        ))
    }
}

// Shared deterministic starts; the public documentation leaves the numerical
// time-series heuristics unspecified. Fixed coefficients are always retained.
pub(in crate::engine) fn initial_model(template: &Model, variance: f64, mean: f64) -> Model {
    let free = parameters(template)
        .into_iter()
        .filter(|p| get(template, *p).is_nan())
        .collect::<Vec<_>>();
    let first_arch = template.arch.iter().position(|x| *x != 0.);
    let first_leverage = template.leverage.iter().position(|x| *x != 0.);
    let mut initial = template.clone();
    initial.fit = None;
    for p in &free {
        let x = match p {
            Parameter::Constant => {
                if template.kind == Kind::Egarch {
                    0.2 * variance.ln()
                } else {
                    0.1 * variance
                }
            }
            Parameter::Garch(_) => {
                0.8 / template.garch.iter().filter(|x| x.is_nan()).count().max(1) as f64
            }
            Parameter::Arch(j) => {
                if template.kind == Kind::Egarch {
                    if Some(*j) == first_arch { 0.1 } else { 0. }
                } else {
                    0.1 / template.arch.iter().filter(|x| x.is_nan()).count().max(1) as f64
                }
            }
            Parameter::Leverage(j) => {
                if Some(*j) == first_leverage {
                    -0.02
                } else {
                    0.
                }
            }
            Parameter::DoF => 10.,
            Parameter::Offset => mean,
        };
        put(&mut initial, *p, x);
    }
    // Accommodate fixed coefficients before applying user-specified starts.
    let margin = 1e-5;
    if template.kind == Kind::Egarch {
        if !egarch_stationary(&initial.garch, margin) {
            for _ in 0..40 {
                for p in &free {
                    if matches!(p, Parameter::Garch(_)) {
                        let v = get(&initial, *p) * 0.5;
                        put(&mut initial, *p, v);
                    }
                }
                if egarch_stationary(&initial.garch, margin) {
                    break;
                }
            }
            if !egarch_stationary(&initial.garch, margin) {
                for p in &free {
                    if matches!(p, Parameter::Garch(_)) {
                        put(&mut initial, *p, 0.);
                    }
                }
            }
        }
    } else {
        for j in 0..initial.q() {
            let l = *initial.leverage.get(j).unwrap_or(&0.);
            if let Some(a) = initial.arch.get_mut(j)
                && template.arch[j].is_nan()
            {
                *a = a.max((-l).max(0.) + 0.01);
            }
        }
        for j in 0..initial.leverage.len() {
            if template.leverage[j].is_nan() {
                let a = *initial.arch.get(j).unwrap_or(&0.);
                initial.leverage[j] = initial.leverage[j].max(-a + 0.001);
            }
        }
        let vol_feasible = |m: &Model| {
            let p: f64 = m.garch.iter().sum::<f64>()
                + m.arch.iter().sum::<f64>()
                + 0.5 * m.leverage.iter().sum::<f64>();
            m.validate().is_ok() && p <= 1. - margin
        };
        if !vol_feasible(&initial) {
            for _ in 0..40 {
                for p in &free {
                    if matches!(
                        p,
                        Parameter::Garch(_) | Parameter::Arch(_) | Parameter::Leverage(_)
                    ) {
                        let v = get(&initial, *p) * 0.5;
                        put(&mut initial, *p, v);
                    }
                }
                if vol_feasible(&initial) {
                    break;
                }
            }
        }
    }
    initial
}

fn egarch_stationary(phi: &[f64], margin: f64) -> bool {
    let mut a: Vec<_> = phi
        .iter()
        .enumerate()
        .map(|(j, x)| x / (1. - margin).powi((j + 1) as i32))
        .collect();
    while let Some(&k) = a.last() {
        if !k.is_finite() || k.abs() >= 1. - 2e-12 {
            return false;
        }
        let n = a.len() - 1;
        a = (0..n)
            .map(|i| (a[i] + k * a[n - 1 - i]) / (1. - k * k))
            .collect();
    }
    true
}

pub(in crate::engine) fn opg_covariance(
    scores: &DMatrix<f64>,
    c: &AtomicBool,
) -> Result<DMatrix<f64>> {
    let k = scores.ncols();
    Ok(if k == 0 {
        DMatrix::zeros(0, 0)
    } else {
        let mut information = DMatrix::zeros(k, k);
        for j in 0..k {
            for r in 0..=j {
                let mut sum = 0.;
                for start in (0..scores.nrows()).step_by(4096) {
                    cancel(c)?;
                    for t in start..(start + 4096).min(scores.nrows()) {
                        sum += scores[(t, j)] * scores[(t, r)];
                    }
                }
                information[(r, j)] = sum;
                information[(j, r)] = sum;
            }
        }
        let identity = DMatrix::identity(k, k);
        if let Some(ch) = information.clone().cholesky() {
            ch.solve(&identity)
        } else {
            information
                .svd(true, true)
                .solve(&identity, 1e-12)
                .map_err(|_| {
                    err(
                        "estimate",
                        "InvalidCovariance",
                        "Singular OPG information matrix.",
                    )
                })?
        }
    })
}
