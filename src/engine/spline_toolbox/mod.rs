//! Curve Fitting Toolbox spline builtins for the Spline Tool app: csapi,
//! csape, spaps, spap2, spmak, fnbrk, aptknt, augknt, fnval (B-form; pp
//! goes to poly_interp), fnder, fnint and fnplt, built from the public
//! function documentation and the published algorithms (de Boor, A
//! Practical Guide to Splines; Reinsch 1967). Free functions only.
mod fit;
mod forms;
mod interp;

use super::poly_interp::{real_matrix, text_of};
use super::{Engine, Execution, Value};
use crate::{Matrix, Result};
use forms::{Bsp, Form, check_knots, parse_form};
use interp::End;

const NAMES: &[&str] = &[
    "csapi", "csape", "spaps", "spap2", "spmak", "fnbrk", "aptknt", "augknt", "fnval", "fnder",
    "fnint", "fnplt",
];
/// Largest number of data sites, knots or evaluation points.
const MAX_POINTS: usize = 8 * 1024 * 1024;
/// Largest spline order.
const MAX_ORDER: usize = 64;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
/// Builtins that act with no output (fnplt plots, fnbrk prints).
pub(super) fn statement(name: &str) -> bool {
    matches!(name, "fnplt" | "fnbrk")
}
fn max_outputs(name: &str) -> usize {
    match name {
        "spaps" => 3,
        "aptknt" | "augknt" | "fnplt" => 2,
        "fnbrk" => 9,
        _ => 1,
    }
}

/// Run `name` with `nout` outputs (0 = statement form); at least one value
/// is returned when nout >= 1.
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    if nout > max_outputs(name) {
        return Err(format!("{name}: too many output arguments"));
    }
    if args.iter().any(|v| matches!(v, Value::Complex(_))) {
        return Err(format!("{name}: complex input is not supported"));
    }
    let one = |v: Result<Value>| v.map(|v| vec![v]);
    match name {
        "csapi" => one(csapi(e, args)),
        "csape" => one(csape(e, args)),
        "spaps" => spaps(e, args, nout),
        "spap2" => one(spap2(e, args)),
        "spmak" => one(spmak(e, args)),
        "fnbrk" => fnbrk(e, args, nout, out),
        "aptknt" => aptknt(e, args, nout),
        "augknt" => augknt(e, args, nout),
        "fnval" => one(fnval(e, args, out)),
        "fnder" => one(fnder(e, args)),
        "fnint" => one(fnint(e, args)),
        _ => fnplt(e, args, nout, out),
    }
}

fn arity(name: &str, args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        return Err(format!("{name}: not enough input arguments"));
    }
    if args.len() > max {
        return Err(format!("{name}: too many input arguments"));
    }
    Ok(())
}
fn vector(v: &Value, who: &str, what: &str) -> Result<Vec<f64>> {
    let m = real_matrix(v, who)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err(format!("{who}: {what} must be a vector"));
    }
    if m.len() > MAX_POINTS {
        return Err(format!("{who}: {what} has more than {MAX_POINTS} entries"));
    }
    Ok(m.data().to_vec())
}
fn finite(v: &[f64], who: &str, what: &str) -> Result<()> {
    if v.iter().all(|x| x.is_finite()) {
        Ok(())
    } else {
        Err(format!("{who}: {what} must be finite"))
    }
}
fn positive_integer(v: &Value, who: &str, what: &str) -> Result<usize> {
    let m = real_matrix(v, who)?;
    match m.data() {
        [k] if *k >= 1. && k.fract() == 0. && *k <= MAX_ORDER as f64 => Ok(*k as usize),
        _ => Err(format!(
            "{who}: {what} must be a positive integer of at most {MAX_ORDER}"
        )),
    }
}
fn function(v: &Value, who: &str) -> Result<Form> {
    parse_form(v, who)?.ok_or_else(|| {
        format!(
            "{who}: the function must be a pp or B-form structure (e.g. from csapi, spaps or spmak)"
        )
    })
}

/// Data values: `ys` series (one per component) of the given length; a
/// vector y is one series, a matrix has one column per site.
struct Data {
    ys: Vec<Vec<f64>>,
    /// y was a vector (outputs follow its orientation).
    vector: bool,
    column: bool,
}
fn data_values(v: &Value, who: &str, sites: &[usize]) -> Result<Data> {
    let y = real_matrix(v, who)?;
    if y.len() > MAX_POINTS * 4 {
        return Err(format!("{who}: y is too large"));
    }
    if y.rows() == 1 || y.cols() == 1 {
        if !sites.contains(&y.len()) {
            return Err(format!(
                "{who}: x and y must have the same number of data sites ({} sites, {} values)",
                sites[0],
                y.len()
            ));
        }
        return Ok(Data {
            ys: vec![y.data().to_vec()],
            vector: true,
            column: y.cols() == 1 && y.rows() > 1,
        });
    }
    if !sites.contains(&y.cols()) {
        return Err(format!(
            "{who}: y must have one column per data site ({}), one row per component; it is {}-by-{}",
            sites[0],
            y.rows(),
            y.cols()
        ));
    }
    let n = y.cols();
    Ok(Data {
        ys: (0..y.rows())
            .map(|r| (0..n).map(|c| y.get(r, c)).collect())
            .collect(),
        vector: false,
        column: false,
    })
}

/// Sorted distinct sites with the series permuted alike.
fn sorted_sites(who: &str, x: &[f64], ys: &[Vec<f64>]) -> Result<(Vec<f64>, Vec<Vec<f64>>)> {
    finite(x, who, "the data sites")?;
    for y in ys {
        finite(y, who, "the data values")?;
    }
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let xs: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
    if xs.windows(2).any(|w| w[0] == w[1]) {
        return Err(format!("{who}: the data sites must be distinct"));
    }
    if xs.len() < 2 {
        return Err(format!("{who}: at least two data sites are needed"));
    }
    let ys = ys
        .iter()
        .map(|y| idx.iter().map(|&i| y[i]).collect())
        .collect();
    Ok((xs, ys))
}

/// Values of `f` at the points of `q`: q's shape for one component, else
/// dim-by-numel(q).
fn evaluate(e: &Engine, f: &Form, q: &Matrix) -> Result<Value> {
    let d = f.dim();
    e.polyopt_admit(q.len() * d)?;
    let mut data = vec![0.; q.len() * d];
    for (i, &x) in q.data().iter().enumerate() {
        if i % 65536 == 0 {
            e.check_cancel()?;
        }
        f.eval_into_checked(x, &mut data[i * d..(i + 1) * d], &|| e.check_cancel())?;
    }
    let (rows, cols) = if d == 1 {
        (q.rows(), q.cols())
    } else {
        (d, q.len())
    };
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

fn csapi(e: &mut Engine, args: &[Value]) -> Result<Value> {
    arity("csapi", args, 2, 3)?;
    if matches!(&args[0], Value::Container(_)) {
        return Err("csapi: gridded data (x as a cell array) is not supported yet".into());
    }
    let x = vector(&args[0], "csapi", "x")?;
    let data = data_values(&args[1], "csapi", &[x.len()])?;
    let (xs, ys) = sorted_sites("csapi", &x, &data.ys)?;
    e.polyopt_admit(xs.len() * (ys.len() * 4 + 8))?;
    let pp = interp::cubic_interpolant("csapi", &xs, &ys, &End::NotAKnot, &End::NotAKnot)?;
    match args.get(2) {
        Some(xx) => evaluate(e, &Form::Pp(pp), &real_matrix(xx, "csapi")?),
        None => pp.to_value(e),
    }
}

/// csape(x, y, conds, valconds): conds a name or [left right] of 0/1/2;
/// end values from y's extra first/last entries or valconds.
fn csape(e: &mut Engine, args: &[Value]) -> Result<Value> {
    arity("csape", args, 2, 4)?;
    if matches!(&args[0], Value::Container(_)) {
        return Err("csape: gridded data (x as a cell array) is not supported yet".into());
    }
    let x = vector(&args[0], "csape", "x")?;
    let n = x.len();
    let data = data_values(&args[1], "csape", &[n, n + 2])?;
    if data.ys.is_empty() {
        return Err("csape: y must contain at least one component".into());
    }
    let len = data.ys[0].len();
    // Explicit end values: y(:, [1 end]) when y carries two extra entries.
    let mut vals: Option<(Vec<f64>, Vec<f64>)> = None;
    let ys: Vec<Vec<f64>> = if len == n + 2 {
        vals = Some((
            data.ys.iter().map(|y| y[0]).collect(),
            data.ys.iter().map(|y| y[n + 1]).collect(),
        ));
        data.ys.iter().map(|y| y[1..=n].to_vec()).collect()
    } else {
        data.ys
    };
    let dim = ys.len();
    if let Some(v) = args.get(3) {
        let m = real_matrix(v, "csape")?;
        if m.len() != 2 && m.len() != 2 * dim {
            return Err("csape: valconds must give one value per end (per component)".into());
        }
        let d = m.data();
        vals = Some(if m.len() == 2 {
            (vec![d[0]; dim], vec![d[1]; dim])
        } else {
            // dim-by-2: column 1 left, column 2 right.
            (d[..dim].to_vec(), d[dim..].to_vec())
        });
    }
    let (xs, ys) = sorted_sites("csape", &x, &ys)?;
    if let Some((l, r)) = &vals {
        finite(l, "csape", "the end condition values")?;
        finite(r, "csape", "the end condition values")?;
    }
    // Per end: 0 Lagrange, 1 slope, 2 second derivative, else named.
    let zero = vec![0.; dim];
    let given = |left: bool| {
        vals.as_ref()
            .map(|(l, r)| if left { l.clone() } else { r.clone() })
    };
    let numeric = |c: f64, left: bool| -> End {
        if c == 1. {
            given(left).map_or(End::Lagrange, End::Slope)
        } else if c == 2. {
            End::Second(given(left).unwrap_or_else(|| zero.clone()))
        } else {
            End::Lagrange
        }
    };
    let (left, right) = match args.get(2) {
        None => (End::Lagrange, End::Lagrange),
        Some(v) => match text_of(v) {
            Some(t) => match t.to_ascii_lowercase().as_str() {
                "complete" | "clamped" => (numeric(1., true), numeric(1., false)),
                "second" => (numeric(2., true), numeric(2., false)),
                "variational" => (End::Second(zero.clone()), End::Second(zero.clone())),
                "not-a-knot" => (End::NotAKnot, End::NotAKnot),
                "periodic" => (End::Periodic, End::Periodic),
                "lagrange" => (End::Lagrange, End::Lagrange),
                other => {
                    return Err(format!(
                        "csape: unknown end condition '{other}'; use 'complete', 'clamped', 'not-a-knot', 'periodic', 'second', 'variational' or [c1 c2] with entries 0, 1 or 2"
                    ));
                }
            },
            None => {
                let m = real_matrix(v, "csape")?;
                match m.data() {
                    [] => (End::Lagrange, End::Lagrange),
                    [a, b] => (numeric(*a, true), numeric(*b, false)),
                    _ => {
                        return Err(
                            "csape: conds must be a name or a two-element vector [c1 c2]".into(),
                        );
                    }
                }
            }
        },
    };
    e.polyopt_admit(xs.len() * (dim * 4 + 8))?;
    let pp = interp::cubic_interpolant("csape", &xs, &ys, &left, &right)?;
    pp.to_value(e)
}

fn spaps(e: &mut Engine, args: &[Value], nout: usize) -> Result<Vec<Value>> {
    arity("spaps", args, 3, 5)?;
    if matches!(&args[0], Value::Container(_)) {
        return Err("spaps: gridded data (x as a cell array) is not supported yet".into());
    }
    let x = vector(&args[0], "spaps", "x")?;
    let n = x.len();
    let data = data_values(&args[1], "spaps", &[n])?;
    finite(&x, "spaps", "the data sites")?;
    for y in &data.ys {
        finite(y, "spaps", "the data values")?;
    }
    let tol = real_matrix(&args[2], "spaps")?;
    let tol = match tol.data() {
        [t] if !t.is_nan() && t.is_finite() => *t,
        [_] => return Err("spaps: tol must be finite".into()),
        _ => {
            return Err(
                "spaps: tol must be a scalar (a tolerance per site range is not supported yet)"
                    .into(),
            );
        }
    };
    let mut w: Option<Vec<f64>> = None;
    let mut m = 2.;
    for v in &args[3..] {
        let mv = real_matrix(v, "spaps")?;
        if mv.len() == 1 && n != 1 {
            m = mv.data()[0];
        } else if mv.is_empty() {
        } else {
            let wv = vector(v, "spaps", "the weights")?;
            if wv.len() != n {
                return Err(format!(
                    "spaps: the weights must have one value per data site ({n}), not {}",
                    wv.len()
                ));
            }
            if wv.iter().any(|v| !v.is_finite() || *v < 0.) {
                return Err("spaps: the weights must be finite and nonnegative".into());
            }
            w = Some(wv);
        }
    }
    if m != 2. {
        return Err(if m == 1. || m == 3. {
            format!("spaps: order m = {m} is not supported yet (m = 2, cubic smoothing, is)")
        } else {
            "spaps: m must be 1, 2 or 3".into()
        });
    }
    if n < 2 {
        return Err("spaps: at least two data sites are needed".into());
    }
    e.polyopt_admit(n * (data.ys.len() * 8 + 16))?;
    let w = w.unwrap_or_else(|| fit::trapezoid_weights(&x));
    let cancel = || e.check_cancel();
    let (pp, rho) = fit::spaps(&x, &data.ys, &w, tol, &cancel)?;
    let sp = fit::cubic_b(&pp);
    let mut outs = vec![sp.to_value(e)?];
    if nout >= 2 {
        let d = data.ys.len();
        let mut vals = vec![0.; n * d];
        for (i, &xi) in x.iter().enumerate() {
            for c in 0..d {
                vals[i * d + c] = pp.eval(xi, c);
            }
        }
        let (r, c) = if !data.vector {
            (d, n)
        } else if data.column {
            (n, 1)
        } else {
            (1, n)
        };
        outs.push(Value::Matrix(Matrix::from_column_vec(r, c, vals)?));
    }
    if nout >= 3 {
        outs.push(Value::Matrix(Matrix::scalar(rho)));
    }
    Ok(outs)
}

fn knot_vector(v: &Value, who: &str) -> Result<Vec<f64>> {
    let t = vector(v, who, "the knots")?;
    finite(&t, who, "the knots")?;
    Ok(t)
}

fn spap2(e: &mut Engine, args: &[Value]) -> Result<Value> {
    arity("spap2", args, 4, 5)?;
    let k = positive_integer(&args[1], "spap2", "the order k")?;
    let x = vector(&args[2], "spap2", "x")?;
    finite(&x, "spap2", "the data sites")?;
    let data = data_values(&args[3], "spap2", &[x.len()])?;
    for y in &data.ys {
        finite(y, "spap2", "the data values")?;
    }
    let t = knot_vector(&args[0], "spap2")?;
    let knots = if t.len() == 1 {
        // l pieces: uniform breaks over the data range.
        let l = t[0];
        if !(l >= 1. && l.fract() == 0. && l <= MAX_POINTS as f64) {
            return Err(
                "spap2: a scalar first argument must be the number of pieces (a positive integer)"
                    .into(),
            );
        }
        let l = l as usize;
        let (lo, hi) = x
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                (a.min(v), b.max(v))
            });
        if hi <= lo {
            return Err("spap2: the data sites must span an interval".into());
        }
        let breaks: Vec<f64> = (0..=l)
            .map(|i| {
                if i == l {
                    hi
                } else {
                    lo + (hi - lo) * i as f64 / l as f64
                }
            })
            .collect();
        forms::clamped_knots(&breaks, k)
    } else {
        let mut t = t;
        t.sort_by(f64::total_cmp);
        t
    };
    check_knots(&knots, k, "spap2")?;
    if knots.len() <= k || knots[0] == knots[knots.len() - 1] {
        return Err(format!(
            "spap2: an order-{k} spline needs at least {} knots spanning an interval",
            k + 1
        ));
    }
    let w = match args.get(4) {
        None => vec![1.; x.len()],
        Some(v) => {
            let w = vector(v, "spap2", "the weights")?;
            if w.len() != x.len() {
                return Err(format!(
                    "spap2: the weights must have one value per data site ({}), not {}",
                    x.len(),
                    w.len()
                ));
            }
            if w.iter().any(|v| !v.is_finite() || *v < 0.) {
                return Err("spap2: the weights must be finite and nonnegative".into());
            }
            w
        }
    };
    e.polyopt_admit(knots.len() * (k + data.ys.len()) + x.len())?;
    let cancel = || e.check_cancel();
    let sp = fit::spap2(knots, k, &x, &data.ys, &w, &cancel)?;
    sp.to_value(e)
}

fn spmak(e: &mut Engine, args: &[Value]) -> Result<Value> {
    arity("spmak", args, 2, 2)?;
    let knots = knot_vector(&args[0], "spmak")?;
    let c = real_matrix(&args[1], "spmak")?;
    finite(c.data(), "spmak", "the coefficients")?;
    // A vector of coefficients is one component; a matrix is dim-by-number.
    let (dim, n) = if c.rows() == 1 || c.cols() == 1 {
        (1, c.len())
    } else {
        (c.rows(), c.cols())
    };
    if n == 0 || knots.len() <= n {
        return Err(format!(
            "spmak: there must be more knots ({}) than coefficients ({n})",
            knots.len()
        ));
    }
    let k = knots.len() - n;
    if k > MAX_ORDER {
        return Err(format!(
            "spmak: the order {k} (knots minus coefficients) exceeds {MAX_ORDER}"
        ));
    }
    check_knots(&knots, k, "spmak")?;
    Bsp {
        knots,
        coefs: c.data().to_vec(),
        order: k,
        dim,
    }
    .to_value(e)
}

const PARTS: &[&str] = &[
    "form", "breaks", "coefs", "pieces", "order", "dim", "knots", "number", "interval",
];

fn part(f: &Form, name: &str) -> Result<Value> {
    let lower = name.to_ascii_lowercase();
    let full = PARTS
        .iter()
        .find(|p| !lower.is_empty() && p.starts_with(lower.as_str()))
        .ok_or_else(|| format!("fnbrk: unknown part '{name}'"))?;
    let row = |v: Vec<f64>| -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
    };
    let num = |v: usize| Value::Matrix(Matrix::scalar(v as f64));
    let (a, b) = f.interval();
    Ok(match (*full, f) {
        ("form", Form::Pp(_)) => Value::Text("pp".into()),
        ("form", Form::B(_)) => Value::Text("B-".into()),
        ("breaks", f) => row(f.breaks())?,
        ("coefs", Form::Pp(p)) => Value::Matrix(p.coef_matrix()?),
        ("coefs", Form::B(s)) => Value::Matrix(s.coef_matrix()?),
        ("pieces", f) => num(f.breaks().len() - 1),
        ("order", Form::Pp(p)) => num(p.order),
        ("order", Form::B(s)) => num(s.order),
        ("dim", f) => num(f.dim()),
        ("knots", Form::B(s)) => row(s.knots.clone())?,
        ("number", Form::B(s)) => num(s.number()),
        ("interval", _) => row(vec![a, b])?,
        (p, _) => {
            return Err(format!("fnbrk: '{p}' is not a part of a pp form"));
        }
    })
}

fn fnbrk(e: &mut Engine, args: &[Value], nout: usize, out: &mut Execution) -> Result<Vec<Value>> {
    arity("fnbrk", args, 1, 10)?;
    let f = function(&args[0], "fnbrk")?;
    let names: Vec<String> = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|v| {
                text_of(v).map(str::to_owned).ok_or_else(|| {
                    "fnbrk: parts are names such as 'knots', 'coefs' or 'order' (restriction to an interval is not supported yet)".to_string()
                })
            })
            .collect::<Result<_>>()?
    } else {
        match &f {
            Form::Pp(_) => ["breaks", "coefs", "pieces", "order", "dim"],
            Form::B(_) => ["knots", "coefs", "number", "order", "dim"],
        }
        .iter()
        .map(|s| s.to_string())
        .collect()
    };
    if nout == 0 && args.len() == 1 {
        // Statement form: describe the function.
        let fmt = |v: &[f64]| {
            v.iter()
                .map(|x| format!("{x:.4}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let text = match &f {
            Form::Pp(p) => format!(
                "The input describes a pp form\n  breaks(1:l+1)\n    {}\n  coefficients(d*l,k): {}-by-{}\n  pieces number l\n    {}\n  order k\n    {}\n  dimension d of target\n    {}\n",
                fmt(&p.breaks),
                p.coefs.len() / p.order,
                p.order,
                p.pieces(),
                p.order,
                p.dim
            ),
            Form::B(s) => format!(
                "The input describes a B- form\n  knots(1:n+k)\n    {}\n  coefficients(d,n)\n    {}\n  number n of coefficients\n    {}\n  order k\n    {}\n  dimension d of target\n    {}\n",
                fmt(&s.knots),
                fmt(&s.coefs),
                s.number(),
                s.order,
                s.dim
            ),
        };
        e.emit(out, text)?;
        return Ok(Vec::new());
    }
    if nout > names.len() {
        return Err("fnbrk: too many output arguments".into());
    }
    // One part per requested output.
    names
        .iter()
        .take(nout.max(1))
        .map(|n| part(&f, n))
        .collect()
}

fn aptknt(e: &mut Engine, args: &[Value], nout: usize) -> Result<Vec<Value>> {
    arity("aptknt", args, 2, 2)?;
    let tau = vector(&args[0], "aptknt", "tau")?;
    finite(&tau, "aptknt", "tau")?;
    if tau.is_empty() {
        return Err("aptknt: tau must not be empty".into());
    }
    if tau.windows(2).any(|w| w[1] < w[0]) {
        return Err("aptknt: tau must be nondecreasing".into());
    }
    let k = positive_integer(&args[1], "aptknt", "the order k")?;
    let (t, k) = fit::aptknt(&tau, k);
    if t[0] == t[t.len() - 1] {
        return Err("aptknt: tau must contain at least two distinct sites".into());
    }
    check_knots(&t, k, "aptknt")
        .map_err(|_| format!("aptknt: some site of tau has multiplicity greater than k = {k}"))?;
    e.polyopt_admit(t.len())?;
    let mut outs = vec![Value::Matrix(Matrix::from_column_vec(1, t.len(), t)?)];
    if nout >= 2 {
        outs.push(Value::Matrix(Matrix::scalar(k as f64)));
    }
    Ok(outs)
}

fn augknt(e: &mut Engine, args: &[Value], nout: usize) -> Result<Vec<Value>> {
    arity("augknt", args, 2, 3)?;
    let t = knot_vector(&args[0], "augknt")?;
    let k = positive_integer(&args[1], "augknt", "the order k")?;
    let mults = match args.get(2) {
        None => vec![1],
        Some(v) => {
            let m = vector(v, "augknt", "mults")?;
            if m.iter()
                .any(|v| !(*v >= 0. && v.fract() == 0. && *v <= k as f64))
            {
                return Err(format!(
                    "augknt: mults must be nonnegative integers of at most k = {k}"
                ));
            }
            m.iter().map(|v| *v as usize).collect()
        }
    };
    // Count the result without expanding its repeated end/interior knots.
    // `fit::augknt` sorts and deduplicates, so do the same small scan here.
    let mut distinct = t.clone();
    distinct.sort_by(f64::total_cmp);
    distinct.dedup();
    if distinct.len() < 2 {
        return Err("augknt: at least two distinct knots are needed".into());
    }
    let interior = distinct.len() - 2;
    if mults.len() != 1 && mults.len() != interior {
        return Err(format!(
            "augknt: mults must be a scalar or have one entry per interior knot ({interior})"
        ));
    }
    let interior_count = if mults.len() == 1 {
        mults[0]
            .checked_mul(interior)
            .ok_or("augknt: result is too large")?
    } else {
        mults.iter().try_fold(0usize, |sum, &m| {
            sum.checked_add(m).ok_or("augknt: result is too large")
        })?
    };
    let result_len = k
        .checked_mul(2)
        .and_then(|n| n.checked_add(interior_count))
        .ok_or("augknt: result is too large")?;
    Matrix::checked_len(1, result_len).map_err(|err| format!("augknt: {err}"))?;
    e.preflight_elements(result_len)
        .map_err(|err| format!("augknt: {err}"))?;
    let (aug, addl) = fit::augknt(&t, k, &mults)?;
    let mut outs = vec![Value::Matrix(Matrix::from_column_vec(1, aug.len(), aug)?)];
    if nout >= 2 {
        outs.push(Value::Matrix(Matrix::scalar(addl as f64)));
    }
    Ok(outs)
}

fn fnval(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let is_struct = |v: &Value| matches!(v, Value::Container(crate::Array::Struct(_)));
    let pp_form = args
        .iter()
        .find(|v| is_struct(v))
        .map(|v| matches!(parse_form(v, "fnval"), Ok(Some(Form::Pp(_)))))
        .unwrap_or(true);
    if pp_form {
        return e
            .polyopt_call("fnval", args, out)?
            .ok_or_else(|| "fnval: could not evaluate the function".to_string());
    }
    let [a, b] = args else {
        return Err(if args.len() < 2 {
            "fnval: not enough input arguments"
        } else {
            "fnval: the left-limit form fnval(f, x, 'l') is not supported yet"
        }
        .into());
    };
    let (f, x) = if is_struct(a) { (a, b) } else { (b, a) };
    if text_of(x).is_some() {
        return Err("fnval: x must be numeric".into());
    }
    let f = function(f, "fnval")?;
    evaluate(e, &f, &real_matrix(x, "fnval")?)
}

fn order_count(v: Option<&Value>, who: &str) -> Result<i64> {
    match v {
        None => Ok(1),
        Some(v) => {
            let m = real_matrix(v, who)?;
            match m.data() {
                [d] if d.fract() == 0. && d.abs() <= MAX_ORDER as f64 => Ok(*d as i64),
                _ => Err(format!(
                    "{who}: the derivative order must be an integer scalar (one variable)"
                )),
            }
        }
    }
}

fn integrate(f: Form, times: usize, value: &[f64]) -> Form {
    let zero = vec![0.; f.dim()];
    let mut f = f;
    for i in 0..times {
        let v = if i + 1 == times { value } else { &zero[..] };
        f = match f {
            Form::Pp(p) => Form::Pp(p.int(v)),
            Form::B(b) => Form::B(b.int(v)),
        };
    }
    f
}

fn fnder(e: &mut Engine, args: &[Value]) -> Result<Value> {
    arity("fnder", args, 1, 2)?;
    let f = function(&args[0], "fnder")?;
    let d = order_count(args.get(1), "fnder")?;
    let g = if d >= 0 {
        match f {
            Form::Pp(p) => Form::Pp(p.der(d as usize)),
            Form::B(b) => Form::B(b.der(d as usize)),
        }
    } else {
        let zero = vec![0.; f.dim()];
        integrate(f, (-d) as usize, &zero)
    };
    g.to_value(e)
}

fn fnint(e: &mut Engine, args: &[Value]) -> Result<Value> {
    arity("fnint", args, 1, 2)?;
    let f = function(&args[0], "fnint")?;
    let d = f.dim();
    let value = match args.get(1) {
        None => vec![0.; d],
        Some(v) => {
            let m = real_matrix(v, "fnint")?;
            match m.len() {
                1 => vec![m.data()[0]; d],
                l if l == d => m.data().to_vec(),
                _ => {
                    return Err(format!(
                        "fnint: the value at the left end must be a scalar or have {d} entries"
                    ));
                }
            }
        }
    };
    let f: Form = match f {
        Form::B(b) if b.order >= MAX_ORDER => {
            return Err(format!("fnint: the order would exceed {MAX_ORDER}"));
        }
        other => other,
    };
    integrate(f, 1, &value).to_value(e)
}

/// fnplt(f, symbol, interval, linewidth) in any order of the options.
fn fnplt(e: &mut Engine, args: &[Value], nout: usize, out: &mut Execution) -> Result<Vec<Value>> {
    arity("fnplt", args, 1, 5)?;
    let f = function(&args[0], "fnplt")?;
    let (mut a, mut b) = f.interval();
    let mut symbol: Option<String> = None;
    let mut width: Option<f64> = None;
    for v in &args[1..] {
        if let Some(t) = text_of(v) {
            if t.eq_ignore_ascii_case("j") || t.eq_ignore_ascii_case("jumps") {
                continue;
            }
            symbol = Some(t.to_owned());
            continue;
        }
        let m = real_matrix(v, "fnplt")?;
        match m.data() {
            [w] => width = Some(*w),
            [lo, hi] if lo < hi && lo.is_finite() && hi.is_finite() => (a, b) = (*lo, *hi),
            [] => {}
            _ => {
                return Err(
                    "fnplt: options are a line symbol, an interval [a b] or a line width".into(),
                );
            }
        }
    }
    let d = f.dim();
    let mut xs: Vec<f64> = (0..=100).map(|i| a + (b - a) * i as f64 / 100.).collect();
    xs[100] = b;
    xs.extend(f.breaks().into_iter().filter(|v| *v > a && *v < b));
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let m = xs.len();
    let samples = Matrix::checked_len(d, m).map_err(|err| format!("fnplt: {err}"))?;
    // Values are both the plotting scratch and the returned component rows.
    // Admit them before the d-by-m allocation (and before the optional copy
    // made for a single return value).
    let required = if nout == 0 {
        samples.checked_mul(2)
    } else {
        // A scalar spline returns [x; y], one more row than its values.
        samples.checked_add(if d == 1 { 2 * m } else { m })
    }
    .ok_or("fnplt: result dimensions are too large")?;
    e.preflight_elements(required)
        .map_err(|err| format!("fnplt: {err}"))?;
    let mut ys = vec![0.; samples];
    for (i, &x) in xs.iter().enumerate() {
        if i % 1024 == 0 {
            e.check_cancel()?;
        }
        f.eval_into_checked(x, &mut ys[i * d..(i + 1) * d], &|| e.check_cancel())?;
    }
    let comp = |c: usize| -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(
            1,
            m,
            (0..m).map(|i| ys[i * d + c]).collect(),
        )?))
    };
    if nout >= 1 {
        // points: [x; y] for a scalar spline, the component rows otherwise;
        // the optional second output is the parameter values.
        let points = if d == 1 {
            let xy = xs.iter().zip(&ys).flat_map(|(x, y)| [*x, *y]).collect();
            Matrix::from_column_vec(2, m, xy)?
        } else {
            Matrix::from_column_vec(d, m, ys)?
        };
        let mut outs = vec![Value::Matrix(points)];
        if nout >= 2 {
            outs.push(Value::Matrix(Matrix::from_column_vec(1, m, xs)?));
        }
        return Ok(outs);
    }
    let mut plot_args = match d {
        1 => vec![Value::Matrix(Matrix::from_column_vec(1, m, xs)?), comp(0)?],
        2 => vec![comp(0)?, comp(1)?],
        3 => vec![comp(0)?, comp(1)?, comp(2)?],
        _ => {
            return Err(format!(
                "fnplt: a function with {d} components cannot be plotted (1, 2 or 3 are)"
            ));
        }
    };
    if let Some(s) = symbol {
        plot_args.push(Value::Text(s));
    }
    if let Some(w) = width {
        plot_args.push(Value::Text("LineWidth".into()));
        plot_args.push(Value::Matrix(Matrix::scalar(w)));
    }
    let name = if d == 3 { "plot3" } else { "plot" };
    e.call(name, plot_args, out)?;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn augknt_and_aptknt_follow_their_definitions() {
        let (t, addl) = fit::augknt(&[1., 2., 3., 3., 3.], 2, &[1]).unwrap();
        assert_eq!(t, vec![1., 1., 2., 3., 3.]);
        assert_eq!(addl, 1);
        let (t, k) = fit::aptknt(&[0., 1., 2.], 4);
        assert_eq!(k, 3);
        assert_eq!(t, vec![0., 0., 0., 2., 2., 2.]);
    }
}
