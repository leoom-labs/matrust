use super::*;
pub(super) fn call(e: &mut Engine, n: &str, a: &[Value]) -> Result<Value> {
    if matches!(n, "de2bi" | "bi2de") {
        return legacy(e, n, a);
    }
    arity(n, a, 2, 5)?;
    let x = numeric(e, &a[0], n)?;
    let class = match &x {
        Array::Typed(t) => Some(t.class()),
        Array::Numeric(_) | Array::Logical(_) => None,
        _ => {
            return Err(err(
                n,
                "InvalidInput",
                "Expected real integer or bit input.",
            ));
        }
    };
    if n == "int2bit" && matches!(x, Array::Logical(_)) {
        return Err(err(
            n,
            "InvalidInput",
            "Logical input is not supported by int2bit.",
        ));
    }
    let max = match class {
        None => 53,
        Some(NumericClass::Single) => 24,
        _ => 64,
    };
    let k = count(e, &a[1], n, 1, max)?;
    let mut pos = 2;
    let msb = if a.get(pos).is_some_and(|v| text(e, v).is_err()) {
        pos += 1;
        boolean(e, &a[pos - 1])?
    } else {
        true
    };
    let o = options(
        e,
        n,
        &a[pos..],
        if n == "bit2int" { &["issigned"] } else { &[] },
    )?;
    let signed = o
        .get("issigned")
        .map(|v| boolean(e, v))
        .transpose()?
        .unwrap_or(false);
    let mut dims = x.dims().to_vec();
    let mut vals = Vec::new();
    let out_class = if n == "int2bit" {
        dims[0] = dims[0].checked_mul(k).ok_or("Dimensions overflow")?;
        let len = preflight(e, &dims, 3)?;
        vals.reserve(len);
        for i in 0..x.len() {
            tick(e, i)?;
            let v = integer(&x, i, n, true)?;
            for j in 0..k {
                vals.push(((v >> if msb { k - 1 - j } else { j }) & 1) as i128);
            }
        }
        match class {
            Some(NumericClass::Single) => Some(NumericClass::Single),
            Some(c) if c.is_signed() => Some(NumericClass::Int8),
            Some(_) => Some(NumericClass::Uint8),
            None => None,
        }
    } else {
        if !dims[0].is_multiple_of(k) {
            return Err(err(
                n,
                "InvalidBitInputSize",
                "The number of rows must be a multiple of n.",
            ));
        }
        dims[0] /= k;
        let len = preflight(e, &dims, 3)?;
        vals.reserve(len);
        for i in 0..len {
            tick(e, i)?;
            let mut v = 0i128;
            for j in 0..k {
                let b = integer(&x, i * k + j, n, false)?;
                if b > 1 {
                    return Err(err(
                        n,
                        "InvalidBitInput",
                        "Input must contain only binary values 0 and 1.",
                    ));
                }
                v |= b << if msb { k - 1 - j } else { j };
            }
            if signed && (v & (1i128 << (k - 1))) != 0 {
                v -= 1i128 << k;
            }
            vals.push(v);
        }
        match class {
            Some(c) if c.is_integer() => {
                let negative = signed || c.is_signed();
                let start = if signed && !c.is_signed() {
                    k.div_ceil(8)
                } else {
                    c.size()
                };
                let cs = if negative {
                    [
                        NumericClass::Int8,
                        NumericClass::Int16,
                        NumericClass::Int32,
                        NumericClass::Int64,
                    ]
                } else {
                    [
                        NumericClass::Uint8,
                        NumericClass::Uint16,
                        NumericClass::Uint32,
                        NumericClass::Uint64,
                    ]
                };
                Some(
                    cs.into_iter()
                        .find(|t| {
                            t.size() >= start && vals.iter().all(|&v| v >= t.min() && v <= t.max())
                        })
                        .ok_or_else(|| {
                            err(
                                n,
                                "Overflow",
                                "No integer output class can contain the result.",
                            )
                        })?,
                )
            }
            v => v,
        }
    };
    if let Some(c) = out_class {
        Ok(Value::from(Array::Typed(
            TypedArray::from_ints(c, dims[0], dims[1..].iter().product(), vals.into_iter())?
                .with_dims(&dims)?,
        )))
    } else {
        output(
            &dims,
            vals.into_iter().map(|v| v as f64).collect(),
            "double",
        )
    }
}
fn legacy(e: &Engine, n: &str, a: &[Value]) -> Result<Value> {
    arity(n, a, 1, if n == "de2bi" { 4 } else { 3 })?;
    let x = numeric(e, &a[0], n)?;
    if x.dims().len() > 2 {
        return Err(err(n, "InvalidInput", "Input must be a vector or matrix."));
    }
    let mut rest = &a[1..];
    let mut left = false;
    if let Some(last) = rest.last()
        && text(e, last).is_ok()
    {
        left = choose(e, n, last, &["left-msb", "right-msb"])? == "left-msb";
        rest = &rest[..rest.len() - 1];
    }
    let (width, p) = if n == "de2bi" {
        if rest.len() > 2 {
            return Err(err(n, "InvalidNumInputs", "Too many input arguments."));
        }
        let w = rest.first().map(|v| array(e, v)).transpose()?;
        let w = if w.as_ref().is_none_or(Array::is_empty) {
            None
        } else {
            Some(count(e, &rest[0], n, 1, 4096)?)
        };
        (
            w,
            rest.get(1)
                .map(|v| count(e, v, n, 2, 65536))
                .transpose()?
                .unwrap_or(2),
        )
    } else {
        if rest.len() > 1 {
            return Err(err(n, "InvalidNumInputs", "Too many input arguments."));
        }
        (
            None,
            rest.first()
                .map(|v| count(e, v, n, 2, 65536))
                .transpose()?
                .unwrap_or(2),
        )
    };
    let mut iv = Vec::with_capacity(x.len());
    for i in 0..x.len() {
        tick(e, i)?;
        iv.push(integer(&x, i, n, false)?);
    }
    let dims;
    let vals = if n == "de2bi" {
        let max = iv.iter().copied().max().unwrap_or(0);
        let mut need = 1;
        let mut t = max;
        while t >= p as i128 {
            t /= p as i128;
            need += 1;
        }
        let k = width.unwrap_or(need);
        if k < need {
            return Err(err(
                n,
                "InvalidNumDigits",
                "Number of columns is too small to represent input values.",
            ));
        }
        dims = vec![x.len(), k];
        preflight(e, &dims, 2)?;
        let mut v = vec![0.; x.len() * k];
        for (i, &number) in iv.iter().enumerate() {
            tick(e, i)?;
            let mut number = number;
            for j in 0..k {
                let col = if left { k - 1 - j } else { j };
                v[col * x.len() + i] = (number % p as i128) as f64;
                number /= p as i128;
            }
        }
        v
    } else {
        dims = vec![x.rows(), 1];
        preflight(e, &dims, 2)?;
        let mut v = vec![0.; x.rows()];
        for (r, out) in v.iter_mut().enumerate() {
            tick(e, r)?;
            let mut number = 0i128;
            for j in 0..x.cols() {
                let c = if left { j } else { x.cols() - 1 - j };
                let b = iv[c * x.rows() + r];
                if b >= p as i128 {
                    return Err(err(n, "InvalidDigit", "Digits must be less than the base."));
                }
                number = number
                    .checked_mul(p as i128)
                    .and_then(|t| t.checked_add(b))
                    .ok_or_else(|| err(n, "Overflow", "Output exceeds exact integer range."))?;
                if number > 9_007_199_254_740_992 {
                    return Err(err(n, "Overflow", "Output exceeds flintmax."));
                }
            }
            *out = number as f64;
        }
        v
    };
    output(&dims, vals, if is_single(&x) { "single" } else { "double" })
}
