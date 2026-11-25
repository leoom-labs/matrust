use super::*;
/// Defuzzify a sampled fuzzy set. MATLAB defines centroid from the samples;
/// bisector divides the piecewise-linear area between adjacent samples.
/// The quadratic area inverse uses a rationalized root to avoid cancellation.
pub(super) fn compute(e: &Engine, x: &[f64], y: &[f64], method: &str) -> Result<f64> {
    if x.is_empty() || x.len() != y.len() {
        return Err("defuzz: x and mf must be nonempty vectors of the same length".into());
    }
    if x.iter().chain(y).any(|v| !v.is_finite())
        || y.iter().any(|&v| v < 0.)
        || x.windows(2).any(|p| p[0] >= p[1])
    {
        return Err(
            "defuzz: expected finite increasing x and finite nonnegative membership values".into(),
        );
    }
    let max = y.iter().copied().fold(0., f64::max);
    if matches!(method, "mom" | "som" | "lom") {
        let mut first = None;
        let mut last = 0.;
        let mut mean = 0.;
        let mut count = 0usize;
        for (i, (&x, &v)) in x.iter().zip(y).enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            if v == max {
                first.get_or_insert(x);
                last = x;
                count += 1;
                // Online mean avoids a sample-sized allocation or a large sum.
                mean = mean * ((count - 1) as f64 / count as f64) + x / count as f64;
            }
        }
        return Ok(match method {
            "som" => first.unwrap(),
            "lom" => last,
            _ => mean,
        });
    }
    if !matches!(method, "centroid" | "bisector") {
        return Err(format!("defuzz: unsupported method '{method}'"));
    }
    if method == "centroid" {
        // MATLAB's documented formula is sum(mf.*x)/sum(mf), not the
        // centroid of the piecewise-linear interpolation. Normalize and
        // shift only to avoid overflowing intermediate products.
        if max == 0. {
            return Err("defuzz: cannot defuzzify an empty membership function (zero area)".into());
        }
        let mut total = 0.;
        let mut moment = 0.;
        for (i, (&point, &membership)) in x.iter().zip(y).enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let membership = membership / max;
            total += membership;
            moment += (point - x[0]) * membership;
        }
        if total <= 0. || !total.is_finite() || !moment.is_finite() {
            return Err("defuzz: zero or nonfinite sampled membership sum".into());
        }
        return Ok(x[0] + moment / total);
    }
    if max == 0. {
        return Err("defuzz: cannot defuzzify an empty membership function (zero area)".into());
    }
    if x.len() == 1 {
        return Ok(x[0]);
    }
    let mut area = 0.;
    for i in 1..x.len() {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        let d = x[i] - x[i - 1];
        let a = y[i - 1] / max;
        let b = y[i] / max;
        let ar = d * (a + b) / 2.;
        area += ar;
    }
    if area <= 0. || !area.is_finite() {
        return Err("defuzz: zero or nonfinite integrated area".into());
    }
    let half = area / 2.;
    let mut accumulated = 0.;
    for i in 1..x.len() {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        let d = x[i] - x[i - 1];
        let a = y[i - 1] / max;
        let b = y[i] / max;
        let ar = d * (a + b) / 2.;
        if ar > 0. && accumulated + ar >= half {
            let target = half - accumulated;
            let root = (a * a + 2. * (b - a) * target / d).max(0.).sqrt();
            let t = if target == 0. {
                0.
            } else {
                2. * target / (a + root)
            };
            return Ok(x[i - 1] + t.clamp(0., d));
        }
        accumulated += ar;
    }
    Ok(*x.last().unwrap())
}
pub(super) fn call(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Value> {
    if a.len() != 3 {
        return Err("defuzz: expected x, membership values and method".into());
    }
    let x = matrix(e, &a[0])?;
    let y = matrix(e, &a[1])?;
    if x.is_nd() || y.is_nd() || (x.rows() > 1 && x.cols() > 1) || (y.rows() > 1 && y.cols() > 1) {
        return Err("defuzz: x and mf must be vectors".into());
    }
    let custom_name = vtext(e, &a[2])?;
    let method = custom_name.to_ascii_lowercase();
    e.charge(x.len())?;
    if !["centroid", "bisector", "mom", "som", "lom"].contains(&method.as_str()) {
        mf::function_name(&a[2].clone().array(&e.cancel)?)?;
        return e.call(&custom_name, vec![a[0].clone(), a[1].clone()], out);
    }
    Ok(Value::Matrix(Matrix::scalar(compute(
        e,
        x.data(),
        y.data(),
        &method,
    )?)))
}
