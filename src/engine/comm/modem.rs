use super::*;
use std::f64::consts::TAU;
#[derive(Clone, Copy, PartialEq)]
enum Family {
    Psk,
    Qam,
    Pam,
    General,
}
/// Locations in geometric order. Cross QAM folds the outer columns of the
/// 2H-by-H rectangle onto the top/bottom arms, preserving the documented labels.
fn geometry(f: Family, m: usize, phase: f64) -> Vec<Complex64> {
    let k = m.ilog2() as usize;
    let h = 1usize << (k / 2);
    let w = m / h;
    let rotation = Complex64::new(phase.cos(), phase.sin());
    (0..m)
        .map(|i| match f {
            Family::Psk => {
                let t = TAU * i as f64 / m as f64 + phase;
                Complex64::new(t.cos(), t.sin())
            }
            Family::Pam => rotation * (2. * i as f64 - (m - 1) as f64),
            _ => {
                let mut x = 2. * (i / h) as f64 - (w - 1) as f64;
                let mut y = (h - 1) as f64 - 2. * (i % h) as f64;
                if k % 2 == 1 && k > 3 && x.abs() > 1.5 * h as f64 {
                    let (ax, ay) = (x.abs(), y.abs());
                    let half = h as f64 / 2.;
                    if ay > half {
                        x = x.signum() * (ax - h as f64);
                        y = y.signum() * (ay + half);
                    } else {
                        x = x.signum() * (2. * h as f64 - ax);
                        y = y.signum() * (1.5 * h as f64 - ay);
                    }
                }
                Complex64::new(x, y)
            }
        })
        .collect()
}
fn gray(x: usize) -> usize {
    x ^ (x >> 1)
}
fn labels(e: &Engine, n: &str, m: usize, f: Family, order: Option<&Value>) -> Result<Vec<usize>> {
    let default = if f == Family::Pam { "bin" } else { "gray" };
    let name = order.map(|v| text(e, v)).transpose();
    if let Ok(name) = name {
        let name = name.unwrap_or_else(|| default.into()).to_ascii_lowercase();
        return match name.as_str() {
            "bin" => Ok((0..m).collect()),
            "gray" if m.is_power_of_two() => {
                let h = 1usize << (m.ilog2() / 2);
                Ok((0..m)
                    .map(|i| {
                        if f == Family::Qam {
                            gray(i / h) * h + gray(i % h)
                        } else {
                            gray(i)
                        }
                    })
                    .collect())
            }
            "gray" => Err(err(
                n,
                "InvalidM",
                "Gray mapping requires M to be a power of 2.",
            )),
            _ => Err(err(
                n,
                "InvalidSymbolOrder",
                "Symbol order must be 'gray', 'bin', or a permutation vector.",
            )),
        };
    }
    if f == Family::Pam {
        return Err(err(
            n,
            "InvalidSymbolOrder",
            "PAM symbol order must be 'gray' or 'bin'.",
        ));
    }
    let x = numeric(e, order.unwrap(), n)?;
    if x.len() != m || (x.rows() != 1 && x.cols() != 1) {
        return Err(err(
            n,
            "InvalidSymbolOrder",
            "Custom symbol order must be a permutation of 0:M-1.",
        ));
    }
    let mut seen = vec![false; m];
    let mut labels = Vec::with_capacity(m);
    for i in 0..m {
        let v = integer(&x, i, n, false)? as usize;
        if v >= m || seen[v] {
            return Err(err(
                n,
                "InvalidSymbolOrder",
                "Custom symbol order must be a permutation of 0:M-1.",
            ));
        }
        seen[v] = true;
        labels.push(v);
    }
    Ok(labels)
}
pub(super) fn call(e: &mut Engine, n: &str, a: &[Value], out: &mut Execution) -> Result<Value> {
    arity(n, a, 2, 18)?;
    let demod = n.ends_with("demod");
    let family = if n.starts_with("gen") {
        Family::General
    } else if n.starts_with("psk") {
        Family::Psk
    } else if n.starts_with("pam") {
        Family::Pam
    } else {
        Family::Qam
    };
    let x = numeric(e, &a[0], n)?;
    let mut pos = 2;
    let mut phase = 0.;
    let mut order = None;
    let mut points;
    let m;
    if family == Family::General {
        let c = numeric(e, &a[1], n)?;
        m = c.len();
        if !(2..=65536).contains(&m) || (c.rows() != 1 && c.cols() != 1) {
            return Err(err(
                n,
                "InvalidConstellation",
                "Constellation must be a vector of 2..65536 points.",
            ));
        }
        e.preflight_elements(m.saturating_mul(8))?;
        points = (0..m).map(|i| complex(&c, i)).collect::<Result<Vec<_>>>()?;
    } else {
        m = count(e, &a[1], n, 2, 65536)?;
        e.preflight_elements(m.saturating_mul(8))?;
        if family != Family::Psk && !m.is_power_of_two() {
            return Err(err(n, "InvalidM", "M must be a power of 2."));
        }
        if family != Family::Qam
            && let Some(v) = a.get(pos)
        {
            if let Ok(t) = text(e, v) {
                if family == Family::Pam
                    || matches!(t.to_ascii_lowercase().as_str(), "gray" | "bin")
                {
                    return Err(err(
                        n,
                        "InvalidPhaseOffset",
                        "Phase offset must be a real scalar or [].",
                    ));
                }
            } else {
                if !array(e, v)?.is_empty() {
                    phase = scalar(e, v)?;
                }
                if !phase.is_finite() {
                    return Err(err(n, "InvalidPhaseOffset", "Phase offset must be finite."));
                }
                pos += 1;
            }
        }
        if let Some(v) = a.get(pos) {
            let t = text(e, v).ok().map(|s| s.to_ascii_lowercase());
            if t.as_deref().is_none_or(|s| matches!(s, "gray" | "bin")) {
                order = Some(v);
                pos += 1;
            }
        }
        points = geometry(family, m, phase);
    }
    let allowed: &[&str] = if family == Family::Pam {
        &[]
    } else if family == Family::General {
        if demod {
            &["outputtype", "noisevariance"]
        } else {
            &["inputtype"]
        }
    } else if demod {
        if family == Family::Qam {
            &[
                "outputtype",
                "noisevariance",
                "outputdatatype",
                "unitaveragepower",
                "plotconstellation",
            ]
        } else {
            &[
                "outputtype",
                "noisevariance",
                "outputdatatype",
                "plotconstellation",
            ]
        }
    } else if family == Family::Qam {
        &[
            "inputtype",
            "outputdatatype",
            "unitaveragepower",
            "plotconstellation",
        ]
    } else {
        &["inputtype", "outputdatatype", "plotconstellation"]
    };
    let o = options(e, n, &a[pos..], allowed)?;
    if points
        .iter()
        .any(|z| !z.re.is_finite() || !z.im.is_finite())
    {
        return Err(err(
            n,
            "InvalidConstellation",
            "Constellation points must be finite.",
        ));
    }
    if o.get("unitaveragepower")
        .map(|v| boolean(e, v))
        .transpose()?
        .unwrap_or(false)
    {
        let scale = (points.iter().map(|z| z.norm_sqr()).sum::<f64>() / m as f64).sqrt();
        for p in &mut points {
            *p /= scale;
        }
    }
    let labels = if family == Family::General {
        (0..m).collect()
    } else {
        labels(e, n, m, family, order)?
    };
    let mut by_symbol = vec![Complex64::new(0., 0.); m];
    for (&label, &point) in labels.iter().zip(&points) {
        by_symbol[label] = point;
    }
    let typ = if let Some(v) = o.get(if demod { "outputtype" } else { "inputtype" }) {
        choose(
            e,
            n,
            v,
            if demod {
                &["integer", "bit", "llr", "approxllr"]
            } else {
                &["integer", "bit"]
            },
        )?
    } else {
        "integer".into()
    };
    if typ != "integer" && !m.is_power_of_two() {
        return Err(err(
            n,
            "InvalidM",
            "Bit and soft output require M to be a power of 2.",
        ));
    }
    let k = m.ilog2() as usize;
    let default_class = if is_single(&x) { "single" } else { "double" };
    let mut class = o
        .get("outputdatatype")
        .map(|v| text(e, v))
        .transpose()?
        .unwrap_or_else(|| default_class.into())
        .to_ascii_lowercase();
    let soft = typ == "llr" || typ == "approxllr";
    if soft {
        class = default_class.into();
    }
    if !demod {
        if class == "single" {
            return Err(err(
                n,
                "UnsupportedDataType",
                "Complex single storage is not supported by this engine; specify OutputDataType='double'.",
            ));
        }
        if class != "double" {
            return Err(err(
                n,
                "InvalidOutputDataType",
                "Modulator OutputDataType must be 'double' or 'single'.",
            ));
        }
    } else if ![
        "double", "single", "int8", "uint8", "int16", "uint16", "int32", "uint32",
    ]
    .contains(&class.as_str())
        && !(typ == "bit" && class == "logical")
    {
        return Err(err(
            n,
            "InvalidOutputDataType",
            "Invalid OutputDataType for the requested OutputType.",
        ));
    }
    if o.get("plotconstellation")
        .map(|v| boolean(e, v))
        .transpose()?
        .unwrap_or(false)
    {
        plot(e, n, &by_symbol, out)?;
    }
    let mut dims = x.dims().to_vec();
    if !demod {
        if typ == "bit" {
            if !dims[0].is_multiple_of(k) {
                return Err(err(
                    n,
                    "InvalidBitInputSize",
                    "Number of input rows must be a multiple of log2(M).",
                ));
            }
            dims[0] /= k;
        }
        let len = preflight(e, &dims, 2)?;
        ComplexMatrix::checked_len(dims[0], dims[1..].iter().product())?;
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            tick(e, i)?;
            let symbol = if typ == "bit" {
                let mut v = 0;
                for j in 0..k {
                    let b = integer(&x, i * k + j, n, false)?;
                    if b > 1 {
                        return Err(err(
                            n,
                            "InvalidBitInput",
                            "Input must contain binary values 0 and 1.",
                        ));
                    }
                    v = (v << 1) | b as usize;
                }
                v
            } else {
                integer(&x, i, n, false)? as usize
            };
            if symbol >= m {
                return Err(err(
                    n,
                    "InvalidSymbol",
                    "Input symbols must be integers in the range 0 to M-1.",
                ));
            }
            data.push(by_symbol[symbol]);
        }
        return complex_output(e, &dims, data);
    }
    let factor = if typ == "integer" { 1 } else { k };
    dims[0] = dims[0].checked_mul(factor).ok_or("Dimensions overflow")?;
    let len = preflight(e, &dims, 2)?;
    let var = if let Some(v) = o.get("noisevariance") {
        let a = numeric(e, v, n)?;
        if a.len() != 1
            && (a.len() != *x.dims().last().unwrap() || (a.rows() != 1 && a.cols() != 1))
        {
            return Err(err(
                n,
                "InvalidNoiseVariance",
                "NoiseVariance must be scalar or a vector matching the last input dimension.",
            ));
        }
        let v = (0..a.len())
            .map(|i| real(&a, i))
            .collect::<Result<Vec<_>>>()?;
        if v.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return Err(err(
                n,
                "InvalidNoiseVariance",
                "NoiseVariance must be finite and positive.",
            ));
        }
        v
    } else {
        vec![1.]
    };
    let rectangular = family == Family::Qam && (m.ilog2().is_multiple_of(2) || m <= 8);
    let qam_scale = if rectangular
        && o.get("unitaveragepower")
            .map(|v| boolean(e, v))
            .transpose()?
            .unwrap_or(false)
    {
        (geometry(family, m, 0.)
            .iter()
            .map(|v| v.norm_sqr())
            .sum::<f64>()
            / m as f64)
            .sqrt()
    } else {
        1.
    };
    let fast = rectangular || family == Family::Psk || family == Family::Pam;
    let work = x.len().saturating_mul(if soft {
        m * k
    } else if fast {
        1
    } else {
        m
    });
    if work > 200_000_000 {
        return Err(err(
            n,
            "WorkLimit",
            "Demodulation exceeds the 200 million distance-operation limit.",
        ));
    }
    e.charge(work)?;
    let mut data = Vec::with_capacity(len);
    let mut distances = vec![0.; if soft { m } else { 0 }];
    let batch_stride = x
        .len()
        .checked_div(*x.dims().last().unwrap())
        .unwrap_or(0)
        .max(1);
    for i in 0..x.len() {
        tick(e, i)?;
        let z = complex(&x, i)?;
        if !z.re.is_finite() || !z.im.is_finite() {
            return Err(err(n, "InvalidInput", "Received symbols must be finite."));
        }
        if soft {
            let variance = var[if var.len() == 1 { 0 } else { i / batch_stride }];
            // Drop |z|^2 before forming distances: it is common to all points.
            // This avoids both overflow and loss of the linear term for large
            // received amplitudes. Then center each bit subset for LogSumExp.
            for (d, &c) in distances.iter_mut().zip(&by_symbol) {
                *d = c.norm_sqr() - 2. * (z.re * c.re + z.im * c.im);
            }
            for bit in (0..k).rev() {
                let mut minimum = [f64::INFINITY; 2];
                for (s, &d) in distances.iter().enumerate() {
                    let b = (s >> bit) & 1;
                    minimum[b] = minimum[b].min(d);
                }
                let mut llr = (minimum[1] - minimum[0]) / variance;
                if typ == "llr" {
                    let mut sums = [0.; 2];
                    for (s, &d) in distances.iter().enumerate() {
                        let b = (s >> bit) & 1;
                        sums[b] += (-(d - minimum[b]) / variance).exp();
                    }
                    llr += sums[0].ln() - sums[1].ln();
                }
                data.push(llr);
            }
        } else {
            let idx = if rectangular {
                let h = 1usize << (m.ilog2() / 2);
                let w = m / h;
                let scale = qam_scale;
                let c = ((z.re * scale + (w - 1) as f64) / 2.)
                    .round()
                    .clamp(0., (w - 1) as f64) as usize;
                let r = (((h - 1) as f64 - z.im * scale) / 2.)
                    .round()
                    .clamp(0., (h - 1) as f64) as usize;
                labels[c * h + r]
            } else if family == Family::Psk {
                let sector =
                    ((z.arg() - phase).rem_euclid(TAU) * m as f64 / TAU).round() as usize % m;
                labels[sector]
            } else if family == Family::Pam {
                let v = z.re * phase.cos() + z.im * phase.sin();
                let idx = ((v + (m - 1) as f64) / 2.)
                    .round()
                    .clamp(0., (m - 1) as f64) as usize;
                labels[idx]
            } else {
                let mut best = 0;
                let mut d = f64::INFINITY;
                for (j, &c) in by_symbol.iter().enumerate() {
                    let ds = (z.re - c.re).hypot(z.im - c.im);
                    if ds < d {
                        d = ds;
                        best = j;
                    }
                }
                best
            };
            if typ == "integer" {
                data.push(idx as f64);
            } else {
                for bit in (0..k).rev() {
                    data.push(((idx >> bit) & 1) as f64);
                }
            }
        }
    }
    output(&dims, data, &class)
}
fn plot(e: &mut Engine, n: &str, c: &[Complex64], out: &mut Execution) -> Result<()> {
    if c.len() > 1024 {
        return Err(err(
            n,
            "PlotLimit",
            "Constellation plots are limited to 1024 labels.",
        ));
    }
    let x = output(&[c.len(), 1], c.iter().map(|z| z.re).collect(), "double")?;
    let y = output(&[c.len(), 1], c.iter().map(|z| z.im).collect(), "double")?;
    e.call("plot", vec![x, y, Value::Text("o".into())], out)?;
    for (i, z) in c.iter().enumerate() {
        e.call(
            "text",
            vec![
                Value::Matrix(Matrix::scalar(z.re)),
                Value::Matrix(Matrix::scalar(z.im)),
                Value::Text(i.to_string()),
            ],
            out,
        )?;
    }
    Ok(())
}
