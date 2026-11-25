use super::*;
pub(in crate::engine) const NAMES: &[&str] = &[
    "trimf", "trapmf", "gaussmf", "gauss2mf", "gbellmf", "sigmf", "dsigmf", "psigmf", "zmf", "smf",
    "pimf", "linsmf", "linzmf",
];
pub(super) fn parameter_count(t: &str) -> Option<usize> {
    match t {
        "trimf" | "gbellmf" => Some(3),
        "trapmf" | "gauss2mf" | "dsigmf" | "psigmf" | "pimf" => Some(4),
        "gaussmf" | "sigmf" | "zmf" | "smf" | "linsmf" | "linzmf" => Some(2),
        "constant" => Some(1),
        _ => None,
    }
}
pub(super) fn function_name(a: &Array) -> Result<String> {
    if let Array::Handle(h) = a {
        return Ok(h.func2str().into());
    }
    let t = string(a)?;
    if t.is_empty()
        || !t.split('.').all(|s| {
            s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    {
        return Err("Expected a membership function name or function handle".into());
    }
    Ok(t)
}
pub(super) fn validate(t: &str, p: &[f64], inputs: Option<usize>) -> Result<()> {
    if p.len() > MAX_ITEMS || p.iter().any(|x| !x.is_finite()) {
        return Err(format!("{t}: parameters must be a finite numeric vector"));
    }
    if let Some(n) = parameter_count(t)
        .or_else(|| (t == "linear").then_some(inputs.map_or(p.len().max(1), |n| n + 1)))
        && p.len() != n
    {
        return Err(format!("{t}: expected {n} parameters"));
    }
    let ordered = match t {
        "trimf" => p[0] <= p[1] && p[1] <= p[2],
        "trapmf" => p[0] <= p[1] && p[2] <= p[3] && p[0] <= p[3],
        "smf" | "zmf" | "linsmf" | "linzmf" => p[0] <= p[1],
        "pimf" => p.windows(2).all(|p| p[0] <= p[1]),
        _ => true,
    };
    if !ordered {
        return Err(format!("{t}: invalid parameter ordering"));
    }
    if (t == "gaussmf" && p[0] < 0.) || (t == "gauss2mf" && (p[0] < 0. || p[2] < 0.)) {
        return Err(format!("{t}: sigma must be nonnegative"));
    }
    if t == "gbellmf" && p[0] == 0. {
        return Err("gbellmf: width must be nonzero".into());
    }
    Ok(())
}
fn sigmoid(x: f64, a: f64, c: f64) -> f64 {
    if a == 0. {
        return 0.5;
    }
    let z = a * (x - c);
    if z >= 0. {
        1. / (1. + (-z).exp())
    } else {
        let v = z.exp();
        v / (1. + v)
    }
}
fn gaussian(x: f64, s: f64, c: f64) -> f64 {
    if s == 0. {
        f64::from(x == c)
    } else {
        (-0.5 * ((x - c) / s).powi(2)).exp()
    }
}
fn s_curve(x: f64, a: f64, b: f64) -> f64 {
    if x >= b {
        1.
    } else if x <= a {
        0.
    } else if x <= a + (b - a) / 2. {
        2. * ((x - a) / (b - a)).powi(2)
    } else {
        1. - 2. * ((x - b) / (b - a)).powi(2)
    }
}
fn rise(x: f64, a: f64, b: f64) -> f64 {
    if x >= b {
        1.
    } else if x <= a {
        0.
    } else {
        (x - a) / (b - a)
    }
}
fn fall(x: f64, a: f64, b: f64) -> f64 {
    if x <= a {
        1.
    } else if x >= b {
        0.
    } else {
        (b - x) / (b - a)
    }
}
/// Published type-1 membership formulae, with crisp limiting shoulders.
pub(super) fn point(t: &str, x: f64, p: &[f64]) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    match t {
        "trimf" => {
            if x == p[1] {
                1.
            } else {
                rise(x, p[0], p[1]).min(fall(x, p[1], p[2]))
            }
        }
        "trapmf" => rise(x, p[0], p[1]).min(fall(x, p[2], p[3])),
        "gaussmf" => gaussian(x, p[0], p[1]),
        "gauss2mf" => {
            let l = if x < p[1] {
                gaussian(x, p[0], p[1])
            } else {
                1.
            };
            let r = if x > p[3] {
                gaussian(x, p[2], p[3])
            } else {
                1.
            };
            l * r
        }
        "gbellmf" => 1. / (1. + ((x - p[2]) / p[0]).abs().powf(2. * p[1])),
        "sigmf" => sigmoid(x, p[0], p[1]),
        "dsigmf" => sigmoid(x, p[0], p[1]) - sigmoid(x, p[2], p[3]),
        "psigmf" => sigmoid(x, p[0], p[1]) * sigmoid(x, p[2], p[3]),
        "smf" => s_curve(x, p[0], p[1]),
        "zmf" => 1. - s_curve(x, p[0], p[1]),
        "pimf" => s_curve(x, p[0], p[1]) * (1. - s_curve(x, p[2], p[3])),
        "linsmf" => rise(x, p[0], p[1]),
        "linzmf" => {
            if p[0] == p[1] {
                f64::from(x < p[0])
            } else {
                fall(x, p[0], p[1])
            }
        }
        "constant" => p[0],
        _ => f64::NAN,
    }
}
pub(super) fn evaluate(e: &mut Engine, t: &str, x: &Value, p: &Array) -> Result<Value> {
    let x = matrix(e, x)?;
    let p = vecnum(p)?;
    validate(t, &p, None)?;
    if x.len().saturating_mul(8) > e.limits.memory_bytes {
        return Err("fuzzy: workspace memory limit exceeded".into());
    }
    e.charge(x.len())?;
    let mut y = Vec::with_capacity(x.len());
    for (i, &v) in x.data().iter().enumerate() {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        y.push(point(t, v, &p));
    }
    Ok(Value::Matrix(
        Matrix::from_column_vec(x.rows(), x.cols(), y)?.with_dims(&x.dims())?,
    ))
}
pub(super) fn record(e: &Engine, t: Array, p: Array, name: &str) -> Result<Record> {
    let typ = function_name(&t)?;
    let params = vecnum(&p)?;
    validate(&typ, &params, None)?;
    Ok(BTreeMap::from([
        ("Name".into(), text(name, e)?),
        ("Type".into(), t),
        ("Parameters".into(), numbers(params)?),
    ]))
}
pub(super) fn constructor(e: &mut Engine, a: &[Value]) -> Result<Value> {
    let mut start = 0;
    let (t, p) = if a.is_empty()
        || (a.len() >= 2 && vtext(e, &a[0]).is_ok_and(|t| t.eq_ignore_ascii_case("Name")))
    {
        (text("trimf", e)?, numbers(vec![0., 0.5, 1.])?)
    } else if a.len() >= 2 {
        start = 2;
        (
            a[0].clone().array(&e.cancel)?,
            a[1].clone().array(&e.cancel)?,
        )
    } else {
        return Err("fismf: specify type and parameters together".into());
    };
    let opts = options(e, &a[start..], &["Name"])?;
    let name = opts
        .get("Name")
        .map(string)
        .transpose()?
        .unwrap_or("mf".into());
    new(e, Object::scalar("fismf", record(e, t, p, &name)?))
}
pub(super) fn evaluate_record(
    e: &mut Engine,
    r: &Record,
    x: &Value,
    out: &mut Execution,
) -> Result<Value> {
    let t = function_name(&r["Type"])?;
    if NAMES.contains(&t.as_str()) || t == "constant" {
        return evaluate(e, &t, x, &r["Parameters"]);
    }
    if t == "linear" {
        return Err("evalmf: linear Sugeno outputs require FIS inputs; use evalfis".into());
    }
    let args = vec![x.clone(), Value::from(r["Parameters"].clone())];
    let result = match &r["Type"] {
        Array::Handle(h) => e.call_handle(h, args, 1, None, out)?.remove(0),
        _ => e.call(&t, args, out)?,
    };
    let m = matrix(e, &result)?;
    let input = matrix(e, x)?;
    if m.dims() != input.dims() {
        return Err("evalmf: custom membership function must preserve input shape".into());
    }
    // A custom function must obey the same missing-value contract.
    if input.data().iter().any(|x| x.is_nan()) {
        let y = m
            .data()
            .iter()
            .zip(input.data())
            .map(|(&y, &x)| if x.is_nan() { f64::NAN } else { y })
            .collect();
        return Ok(Value::Matrix(
            Matrix::from_column_vec(m.rows(), m.cols(), y)?.with_dims(&m.dims())?,
        ));
    }
    Ok(result)
}
pub(super) fn evalmf(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Value> {
    if a.len() == 3 {
        let r = record(
            e,
            a[2].clone().array(&e.cancel)?,
            a[1].clone().array(&e.cancel)?,
            "mf",
        )?;
        return evaluate_record(e, &r, &a[0], out);
    }
    if a.len() != 2 {
        return Err("evalmf: expected (mf,x) or (x,parameters,type)".into());
    }
    let o = object(e, &a[0])?;
    if o.class != "fismf" {
        return Err("evalmf: expected fismf objects".into());
    }
    if o.len() == 1 {
        return evaluate_record(e, o.record()?, &a[1], out);
    }
    let x = matrix(e, &a[1])?;
    if x.rows() > 1 && x.cols() > 1 || x.is_nd() {
        return Err("evalmf: multiple membership functions require scalar or vector x".into());
    }
    Matrix::checked_len(x.len(), o.len())?;
    if x.len().saturating_mul(o.len()).saturating_mul(8) > e.limits.memory_bytes {
        return Err("evalmf: workspace memory limit exceeded".into());
    }
    let mut y = Vec::with_capacity(x.len() * o.len());
    for r in o.items.iter() {
        let v = evaluate_record(e, r, &a[1], out)?;
        y.extend_from_slice(matrix(e, &v)?.data());
    }
    Ok(Value::Matrix(Matrix::from_column_vec(x.len(), o.len(), y)?))
}
