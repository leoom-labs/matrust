use super::*;
pub(super) fn call(e: &Engine, n: &str, a: &[Value]) -> Result<Vec<Value>> {
    let bit = n == "biterr";
    arity(n, a, 2, if bit { 4 } else { 3 })?;
    let x = numeric(e, &a[0], n)?;
    let y = numeric(e, &a[1], n)?;
    if x.dims().len() > 2 || y.dims().len() > 2 || x.is_empty() || y.is_empty() {
        return Err(err(
            n,
            "InvalidSize",
            "Inputs must be nonempty vectors or matrices.",
        ));
    }
    let mut idx = 2;
    let explicit_k = if bit && a.get(idx).is_some_and(|v| text(e, v).is_err()) {
        idx += 1;
        if array(e, &a[idx - 1])?.is_empty() {
            None
        } else {
            Some(count(e, &a[idx - 1], n, 1, 64)?)
        }
    } else {
        None
    };
    let same = x.rows() == y.rows() && x.cols() == y.cols();
    let (rows, cols) = (x.rows().max(y.rows()), x.cols().max(y.cols()));
    let row = !same && (x.rows() == 1 || y.rows() == 1) && x.cols() == y.cols();
    let col = !same && (x.cols() == 1 || y.cols() == 1) && x.rows() == y.rows();
    if !same && !row && !col {
        return Err(err(
            n,
            "InvalidSize",
            "Inputs must have the same size, or a vector must match the rows or columns of a matrix.",
        ));
    }
    let flag = if let Some(v) = a.get(idx) {
        idx += 1;
        choose(e, n, v, &["overall", "row-wise", "column-wise"])?
    } else {
        if row {
            "row-wise"
        } else if col {
            "column-wise"
        } else {
            "overall"
        }
        .into()
    };
    if idx != a.len() || (row && flag == "column-wise") || (col && flag == "row-wise") {
        return Err(err(
            n,
            "InvalidComparison",
            "Comparison flag is incompatible with input dimensions.",
        ));
    }
    preflight(e, &[rows, cols], 4)?;
    let mut max = 0i128;
    if bit {
        for v in [&x, &y] {
            for i in 0..v.len() {
                tick(e, i)?;
                max = max.max(integer(v, i, n, false)?);
            }
        }
    }
    let needed = ((128 - max.leading_zeros()) as usize).max(1);
    let k = explicit_k.unwrap_or(needed);
    if bit && needed > k {
        return Err(err(
            n,
            "InvalidK",
            "k is too small to represent the largest input value.",
        ));
    }
    let outdims = match flag.as_str() {
        "row-wise" => [rows, 1],
        "column-wise" => [1, cols],
        _ => [1, 1],
    };
    let mut sums = vec![0.; outdims[0] * outdims[1]];
    let mut individual = Vec::with_capacity(rows * cols);
    for c in 0..cols {
        for r in 0..rows {
            let i = c * rows + r;
            tick(e, i)?;
            let xi =
                if x.rows() == 1 { 0 } else { r } + if x.cols() == 1 { 0 } else { c * x.rows() };
            let yi =
                if y.rows() == 1 { 0 } else { r } + if y.cols() == 1 { 0 } else { c * y.rows() };
            let v = if bit {
                (integer(&x, xi, n, false)? ^ integer(&y, yi, n, false)?).count_ones() as f64
            } else {
                f64::from(complex(&x, xi)? != complex(&y, yi)?)
            };
            individual.push(v);
            sums[match flag.as_str() {
                "row-wise" => r,
                "column-wise" => c,
                _ => 0,
            }] += v;
        }
    }
    // Rates divide by the comparisons represented by each count. The public
    // symerr worked example gives 3/4 for overall matrix/vector broadcasting;
    // its generic "smaller input" prose is inconsistent with that example.
    // Octave communications independently agrees with these slice denominators.
    let denom = match flag.as_str() {
        "row-wise" => cols,
        "column-wise" => rows,
        _ => rows * cols,
    };
    let denom = denom as f64 * if bit { k as f64 } else { 1. };
    let class = if bit && is_single(&x) && is_single(&y) {
        "single"
    } else {
        "double"
    };
    Ok(vec![
        output(&outdims, sums.clone(), class)?,
        output(
            &outdims,
            sums.into_iter().map(|v| v / denom).collect(),
            class,
        )?,
        output(
            &[rows, cols],
            individual,
            if bit { class } else { "logical" },
        )?,
    ])
}
