//! Resampling using session random draws; Efraimidis & Spirakis (2006)
//! weighted keys in log space. Uniform subsets delegate to randperm(n,k).
use super::*;
use crate::engine::stats_dist::families::WalkCache;
fn select(e: &Engine, v: &Value, r: usize, c: usize, positions: &[usize]) -> Result<Value> {
    Matrix::checked_len(r, c)?;
    let a = v.clone().array(&e.cancel)?;
    macro_rules! gather {
        ($f:expr) => {{
            let mut out = Vec::with_capacity(positions.len());
            for (i, &p) in positions.iter().enumerate() {
                poll(e, i)?;
                out.push(($f)(p));
            }
            out
        }};
    }
    Ok(match a {
        Array::Numeric(m) => matrix(r, c, gather!(|p: usize| m.data()[p]))?,
        Array::Typed(t) => Value::Container(Array::Typed(
            t.select_with_cancel(r, c, positions, &e.cancel)?,
        )),
        Array::Logical(m) => Value::Logical(LogicalMatrix::from_column_vec(
            r,
            c,
            gather!(|p: usize| m.data()[p]),
            &e.cancel,
        )?),
        Array::Character(m) => {
            Value::Container(Array::Character(crate::CharacterRow::from_column_major(
                r,
                c,
                gather!(|p: usize| m.units()[p]),
                &e.cancel,
            )?))
        }
        Array::Str(m) => Value::Container(Array::Str(crate::StringArray::new(
            r,
            c,
            gather!(|p: usize| m.data()[p].clone()),
        )?)),
        Array::Cell(m) => {
            let mut out = Vec::with_capacity(positions.len());
            for (i, &p) in positions.iter().enumerate() {
                poll(e, i)?;
                out.push(m.get_linear(p)?.clone())
            }
            Value::Container(Array::Cell(crate::CellArray::from_shared(
                r, c, out, &e.cancel,
            )?))
        }
        _ => {
            return Err(
                "Sampling supports real numeric, logical, char, string and cell data".into(),
            );
        }
    })
}
fn weights(e: &Engine, v: Option<&Value>, n: usize) -> Result<Option<Vec<f64>>> {
    let Some(v) = v.filter(|v| !empty(v)) else {
        return Ok(None);
    };
    let m = mat(v, e)?;
    if m.len() != n || m.rows() > 1 && m.cols() > 1 {
        return Err("Weights must be a vector matching the population".into());
    }
    if m.data().iter().any(|w| !w.is_finite() || *w < 0.) {
        return Err("Weights must be finite and nonnegative".into());
    }
    let max = m.data().iter().copied().fold(0., f64::max);
    if max == 0. {
        return Err("Weights must contain positive mass".into());
    }
    Ok(Some(m.data().iter().map(|w| w / max).collect()))
}
fn boolean(e: &Engine, v: &Value) -> Result<bool> {
    if let Some(s) = text(v) {
        match s.to_ascii_lowercase().as_str() {
            "on" | "true" => return Ok(true),
            "off" | "false" | "auto" => return Ok(false),
            _ => return Err("Expected true, false, on, off or auto".into()),
        }
    }
    match scalar(v, e)? {
        0. => Ok(false),
        1. => Ok(true),
        _ => Err("Expected true or false".into()),
    }
}
fn draw(
    e: &mut Engine,
    n: usize,
    k: usize,
    replace: bool,
    w: Option<&[f64]>,
    out: &mut Execution,
) -> Result<Vec<usize>> {
    Matrix::checked_len(k, 1)?;
    if n == 0 && k > 0 {
        return Err("Cannot sample an empty population".into());
    }
    if !replace && k > n {
        return Err("Sample size exceeds population without replacement".into());
    }
    if !replace && w.is_none() {
        let v = e.call("randperm", vec![val(n as f64), val(k as f64)], out)?;
        return Ok(mat(&v, e)?.data().iter().map(|v| *v as usize - 1).collect());
    }
    if !replace {
        let w = w.unwrap();
        if w.iter().filter(|v| **v > 0.).count() < k {
            return Err("Not enough positive weights for sample without replacement".into());
        }
        let mut keys = Vec::with_capacity(n);
        for (i, &w) in w.iter().enumerate() {
            poll(e, i)?;
            let u = e.random.next_f64();
            if w > 0. {
                keys.push((u.ln() / w, i))
            }
        }
        sort(e, &mut keys, |a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)))?;
        return Ok(keys.into_iter().take(k).map(|(_, i)| i).collect());
    }
    let cumulative = w.map(|w| {
        let mut sum = 0.;
        w.iter()
            .map(|v| {
                sum += v;
                sum
            })
            .collect::<Vec<_>>()
    });
    let mut idx = Vec::with_capacity(k);
    for i in 0..k {
        poll(e, i)?;
        let u = e.random.next_f64();
        idx.push(if let Some(c) = &cumulative {
            c.partition_point(|v| *v <= u * c[c.len() - 1]).min(n - 1)
        } else {
            ((u * n as f64) as usize).min(n.saturating_sub(1))
        })
    }
    Ok(idx)
}
fn slice(e: &Engine, data: &Value, idx: &[usize], dim: usize) -> Result<Value> {
    let (r, c) = data.shape(&e.cancel)?;
    let (rr, cc) = if dim == 1 {
        (idx.len(), c)
    } else {
        (r, idx.len())
    };
    let n = Matrix::checked_len(rr, cc)?;
    let mut positions = Vec::with_capacity(n);
    for j in 0..cc {
        for i in 0..rr {
            poll(e, i + j * rr)?;
            positions.push(if dim == 1 {
                idx[i] + j * r
            } else {
                i + idx[j] * r
            })
        }
    }
    select(e, data, rr, cc, &positions)
}
pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "randsample" => sample(e, args, out),
        "datasample" => data(e, args, out),
        "mnrnd" => multinomial(e, args),
        _ => bootstrap(e, args, out),
    }
}
/// `mnrnd(n,p[,m])`: row i holds category counts of n(i) trials with
/// probabilities p(i,:). Every input is checked before the first draw. A row
/// is drawn as conditional binomials (count j from the trials left with
/// p(j) over the remaining mass) through the binornd sampler: at most k-1
/// binomial draws per row and no array of individual trial outcomes.
fn multinomial(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    const COUNTS: &str =
        "mnrnd: N must be a scalar or column vector of nonnegative integers below 2^53";
    const PROBS: &str = "mnrnd: P must contain finite nonnegative probabilities";
    if !(2..=3).contains(&args.len()) {
        return Err("mnrnd: expected N, P and optional M".into());
    }
    let n = mat(&args[0], e).map_err(|_| COUNTS)?;
    if n.cols() != 1 && n.len() != 1 {
        return Err(COUNTS.into());
    }
    for (i, &v) in n.data().iter().enumerate() {
        poll(e, i)?;
        if !((0. ..9_007_199_254_740_992.).contains(&v) && v.fract() == 0.) {
            return Err(COUNTS.into());
        }
    }
    let p = mat(&args[1], e).map_err(|_| PROBS)?;
    let (rp, k) = (p.rows(), p.cols());
    if k == 0 {
        return Err("mnrnd: P must have at least one column".into());
    }
    let rn = n.len();
    let rows = if let Some(m) = args.get(2) {
        let m = integer(m, e).map_err(|_| "mnrnd: M must be a nonnegative integer scalar")?;
        if (rn != 1 && rn != m) || (rp != 1 && rp != m) {
            return Err("mnrnd: M must match the rows of N and P".into());
        }
        m
    } else {
        match (rn, rp) {
            (1, r) | (r, 1) => r,
            (a, b) if a == b => a,
            _ => return Err("mnrnd: the length of N must match the number of rows of P".into()),
        }
    };
    let total = Matrix::checked_len(rows, k)?;
    let pd = p.data();
    for i in 0..rp {
        let mut sum = 0.;
        for j in 0..k {
            poll(e, i * k + j)?;
            let v = pd[i + j * rp];
            if !(v.is_finite() && v >= 0.) {
                return Err(PROBS.into());
            }
            sum += v;
        }
        // Octave's mnrnd tolerance; the draws use each row's own mass.
        if (sum - 1.).abs() > 1e-6 {
            return Err("mnrnd: each row of P must sum to 1".into());
        }
    }
    let mut counts = vec![0.; total];
    let mut tail = vec![0.; k];
    let mut cache = WalkCache::default();
    let mut step = 0;
    for i in 0..rows {
        let pi = if rp == 1 { 0 } else { i };
        let mut left = n.data()[if rn == 1 { 0 } else { i }];
        let mut mass = 0.;
        let mut last = 0;
        for j in (0..k).rev() {
            poll(e, step)?;
            step += 1;
            let v = pd[pi + j * rp];
            if v > 0. && mass == 0. {
                last = j;
            }
            mass += v;
            tail[j] = mass;
        }
        for j in 0..k {
            poll(e, step)?;
            step += 1;
            let v = pd[pi + j * rp];
            if left == 0. {
                break;
            }
            if v == 0. {
                continue;
            }
            // tail[j] >= v > 0, so the ratio lies in (0,1].
            let x = if j == last {
                left
            } else {
                Family::Bino.rnd(&[left, (v / tail[j]).min(1.)], &mut e.random, &mut cache)
            };
            if !(0. ..=left).contains(&x) {
                return Err("mnrnd: binomial draw failed".into());
            }
            counts[i + j * rows] = x;
            left -= x;
        }
    }
    Ok(vec![matrix(rows, k, counts)?])
}
fn sample(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if !(2..=4).contains(&args.len()) {
        return Err("randsample: expected population, k, replacement and weights".into());
    }
    if matches!(&args[0], Value::Container(Array::Cell(_))) {
        return Err("randsample: cell population is not supported".into());
    }
    let (r, c) = args[0].shape(&e.cancel)?;
    if r > 1 && c > 1 {
        return Err("randsample: population must be a vector".into());
    }
    let scalar_population = r * c == 1
        && matches!(
            &args[0],
            Value::Matrix(_) | Value::Container(Array::Typed(_))
        );
    let n = if scalar_population {
        integer(&args[0], e)?
    } else {
        r * c
    };
    let k = integer(&args[1], e)?;
    Matrix::checked_len(k, 1)?;
    let replace = args.get(2).map_or(Ok(false), |v| boolean(e, v))?;
    let w = weights(e, args.get(3), n)?;
    if !replace && w.is_some() {
        return Err("randsample: weights require replacement".into());
    }
    let idx = draw(e, n, k, replace, w.as_deref(), out)?;
    let v = if scalar_population {
        matrix(k, 1, idx.iter().map(|i| (i + 1) as f64).collect())?
    } else if r == 1 {
        select(e, &args[0], 1, k, &idx)?
    } else {
        select(e, &args[0], k, 1, &idx)?
    };
    Ok(vec![v])
}
fn data(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if args.len() < 2 {
        return Err("datasample: expected data and k".into());
    }
    let (end, o) = Opts::parse(args, 2, &["Replace", "Weights"])?;
    if end > 3 {
        return Err("datasample: invalid options".into());
    }
    let (r, c) = args[0].shape(&e.cancel)?;
    let dim = if end == 3 {
        integer(&args[2], e)?
    } else if r == 1 {
        2
    } else {
        1
    };
    if dim != 1 && dim != 2 {
        return Err("datasample: only dimensions 1 and 2 supported yet".into());
    }
    let n = if dim == 1 { r } else { c };
    let k = integer(&args[1], e)?;
    Matrix::checked_len(if dim == 1 { k } else { r }, if dim == 1 { c } else { k })?;
    let replace = o.get("replace").map_or(Ok(true), |v| boolean(e, v))?;
    let w = weights(e, o.get("weights"), n)?;
    let idx = draw(e, n, k, replace, w.as_deref(), out)?;
    Ok(vec![
        slice(e, &args[0], &idx, dim)?,
        if dim == 1 {
            matrix(k, 1, idx.iter().map(|i| (i + 1) as f64).collect())?
        } else {
            row(idx.iter().map(|i| (i + 1) as f64).collect())?
        },
    ])
}
fn bootstrap(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if args.len() < 3 {
        return Err("bootstrp: expected nboot, bootfun and data".into());
    }
    let (end, o) = Opts::parse(args, 3, &["Weights", "Options"])?;
    let nb = integer(&args[0], e)?;
    let handle = match &args[1] {
        Value::Container(Array::Handle(h)) => Some(h.clone()),
        v if empty(v) => None,
        _ => return Err("bootstrp: bootfun must be a handle or []".into()),
    };
    if let Some(v) = o.get("options") {
        let Value::Container(Array::Struct(s)) = v else {
            return Err("bootstrp: Options must be a scalar struct".into());
        };
        if s.len() != 1 {
            return Err("bootstrp: Options must be a scalar struct".into());
        }
        for (i, n) in s.field_names().iter().enumerate() {
            if !n.eq_ignore_ascii_case("UseParallel") {
                return Err(format!("bootstrp: unsupported Options field {n}"));
            }
            boolean(e, &Value::Container(s.get(i, 0)?.as_ref().clone()))?;
        }
    }
    let mut n = None;
    for v in &args[2..end] {
        let (r, c) = v.shape(&e.cancel)?;
        if r * c != 1 {
            if n.is_some_and(|n| n != r) {
                return Err("bootstrp: data must have equal row counts".into());
            }
            n = Some(r)
        }
    }
    let n = n.unwrap_or(1);
    let total = nb
        .checked_mul(n)
        .filter(|n| *n <= 100_000_000)
        .ok_or("bootstrp: index array too large")?; // Check before drawing or allocating.
    Matrix::checked_len(n, nb).map_err(|_| "bootstrp: index array too large")?;
    let w = weights(e, o.get("weights"), n)?;
    let indices = draw(e, n, total, true, w.as_deref(), out)?;
    let bootsam = matrix(n, nb, indices.iter().map(|i| (i + 1) as f64).collect())?;
    let Some(h) = handle else {
        return Ok(vec![blank(), bootsam]);
    };
    let mut results = Vec::new();
    let mut width = None;
    for b in 0..nb {
        e.check_cancel()?;
        let mut data = Vec::new();
        for v in &args[2..end] {
            let (r, c) = v.shape(&e.cancel)?;
            data.push(if r * c == 1 {
                v.clone()
            } else {
                slice(e, v, &indices[b * n..(b + 1) * n], 1)?
            })
        }
        let result = e
            .call_handle(&h, data, 1, None, out)?
            .into_iter()
            .next()
            .ok_or("bootstrp: the function returned no value")?;
        let m = mat(&result, e)?;
        if let Some(k) = width {
            if m.len() != k {
                return Err("bootstrp: bootfun output size changed".into());
            }
        } else {
            Matrix::checked_len(nb, m.len()).map_err(|_| "bootstrp: bootstat array too large")?;
            width = Some(m.len());
            results.reserve(nb * m.len());
        }
        results.extend_from_slice(m.data());
    }
    let width = width.unwrap_or(0);
    let mut cm = vec![0.; results.len()];
    for b in 0..nb {
        for j in 0..width {
            poll(e, b * width + j)?;
            cm[b + j * nb] = results[b * width + j]
        }
    }
    Ok(vec![matrix(nb, width, cm)?, bootsam])
}
