use super::*;
pub(super) fn fsk(e: &mut Engine, a: &[Value], demod: bool) -> Result<Value> {
    let name = if demod { "fskdemod" } else { "fskmod" };
    arity(a, 4, if demod { 6 } else { 7 }, name)?;
    let x = numeric(e, &a[0], name, 2)?;
    let m = order(e, &a[1], name)?;
    let sep = positive(e, &a[2], name)?;
    let ns = integer(e, &a[3], 2, MAX_SAMPLES, name)?;
    let fs = if let Some(v) = a.get(4) {
        positive(e, v, name)?
    } else {
        1.
    };
    if (m - 1) as f64 * sep > fs {
        return Err(format!("{name}: (M-1)*freqsep must not exceed Fs"));
    }
    let cont = if demod {
        true
    } else {
        choice(e, a.get(5), "cont", &["cont", "discont"], name)? == "cont"
    };
    let gray = choice(
        e,
        a.get(if demod { 5 } else { 6 }),
        "bin",
        &["bin", "gray"],
        name,
    )? == "gray";
    let (nr, nc) = lanes(&x);
    if demod && nr % ns != 0 {
        return Err("fskdemod: signal length must be a multiple of nsamp".into());
    }
    if !demod {
        symbols(e, &x, m, name)?;
    }
    let rows = if demod {
        nr / ns
    } else {
        nr.checked_mul(ns).ok_or("fskmod: output size overflow")?
    };
    let len = rows
        .checked_mul(nc)
        .filter(|n| *n <= MAX_SAMPLES)
        .ok_or("fsk: output sample limit exceeded")?;
    let cost = if demod {
        x.len().saturating_mul(m)
    } else {
        len
    };
    reserve(e, len.saturating_mul(if demod { 1 } else { 2 }))?;
    work(e, cost)?;
    let omega = |k: usize| TAU * (k as f64 - (m - 1) as f64 / 2.) * (sep / fs);
    if demod {
        let mut y = zeros::<f64>(e, len)?;
        for c in 0..nc {
            for s in 0..rows {
                e.check_cancel()?;
                // A common positive scale leaves tone energies ordered while
                // avoiding squared-correlation overflow and underflow.
                let scale = symbol_scale(e, &x, c * nr + s * ns, ns)?;
                if scale == 0. {
                    continue;
                }
                let mut best = 0;
                let mut score = -1.;
                for k in 0..m {
                    let mut acc = Complex64::new(0., 0.);
                    let w = omega(k);
                    for t in 0..ns {
                        if t % 4096 == 0 {
                            e.check_cancel()?;
                        }
                        acc += (at(&x, c * nr + s * ns + t) / scale) * unit(-w * t as f64);
                    }
                    let energy = acc.norm_sqr();
                    if energy > score {
                        score = energy;
                        best = k;
                    }
                }
                y[c * rows + s] = label(best, gray) as f64;
            }
        }
        let dims = if x.rows() == 1 { (1, len) } else { (rows, nc) };
        rv(dims.0, dims.1, y)
    } else {
        let mut y = zeros::<Complex64>(e, len)?;
        for c in 0..nc {
            let mut phase = 0.;
            for s in 0..nr {
                e.check_cancel()?;
                let sym = at(&x, c * nr + s).re as usize;
                let w = omega(if gray { ungray(sym) } else { sym });
                // Independent oscillators keep their absolute time in discont mode.
                let start = if cont {
                    phase
                } else {
                    (w * (s * ns) as f64).rem_euclid(TAU)
                };
                for t in 0..ns {
                    if t % 4096 == 0 {
                        e.check_cancel()?;
                    }
                    y[c * rows + s * ns + t] = unit(start + w * t as f64);
                }
                phase = (start + w * ns as f64).rem_euclid(TAU);
            }
        }
        cv(
            e,
            &if x.rows() == 1 {
                vec![1, len]
            } else {
                vec![rows, nc]
            },
            y,
        )
    }
}
pub(super) fn dpsk(e: &mut Engine, a: &[Value], demod: bool) -> Result<Value> {
    let name = if demod { "dpskdemod" } else { "dpskmod" };
    arity(a, 2, 4, name)?;
    let x = numeric(e, &a[0], name, 2)?;
    if x.len() > MAX_SAMPLES {
        return Err(format!("{name}: sample limit exceeded"));
    }

    let m = order(e, &a[1], name)?;
    let rotation = match a.get(2) {
        Some(v) if !empty(v) => scalar(e, v, name)?,
        _ => 0.,
    };
    let gray = choice(e, a.get(3), "bin", &["bin", "gray"], name)? == "gray";
    if !demod {
        symbols(e, &x, m, name)?;
    }
    reserve(e, x.len().saturating_mul(2))?;
    work(e, x.len())?;
    let (nr, nc) = lanes(&x);
    if demod {
        let mut y = zeros::<f64>(e, x.len())?;
        for c in 0..nc {
            let mut prev = 0.;
            for i in 0..nr {
                if i % 4096 == 0 {
                    e.check_cancel()?;
                }
                let phase = at(&x, c * nr + i).arg();
                // Subtract phases directly: products of finite samples can
                // otherwise overflow or underflow before atan2 is evaluated.
                let p = phase - prev - rotation;
                let k = (p.rem_euclid(TAU) * m as f64 / TAU).round() as usize % m;
                y[c * nr + i] = label(k, gray) as f64;
                prev = phase;
            }
        }
        rv(x.rows(), x.cols(), y)
    } else {
        let mut y = zeros::<Complex64>(e, x.len())?;
        for c in 0..nc {
            let mut phase = 0.;
            for i in 0..nr {
                if i % 4096 == 0 {
                    e.check_cancel()?;
                }
                let k = at(&x, c * nr + i).re as usize;
                phase =
                    (phase + rotation + TAU * (if gray { ungray(k) } else { k }) as f64 / m as f64)
                        .rem_euclid(TAU);
                y[c * nr + i] = unit(phase);
            }
        }
        cv(e, &x.dims(), y)
    }
}
fn phases(e: &Engine, v: Option<&Value>, nc: usize) -> Result<Vec<usize>> {
    let Some(v) = v.filter(|v| !empty(v)) else {
        return zeros(e, nc);
    };
    let p = vector(e, v, "msk initphase")?;
    if p.rows() != 1 || p.len() != nc {
        return Err("msk: initphase must be a row vector with one value per channel".into());
    }
    p.data()
        .iter()
        .map(|v| {
            let q = *v / FRAC_PI_2;
            if (q - q.round()).abs() > 1e-10 {
                Err("msk: initphase must contain integer multiples of pi/2".into())
            } else {
                Ok(q.round().rem_euclid(4.) as usize)
            }
        })
        .collect()
}
// OQPSK half-sine precoding: a bit determines the sign on the next I/Q axis.
// Differential MSK directly selects the +/- pi/2 phase increment (CPFSK h=1/2).
fn msk_next(q: usize, bit: bool, diff: bool) -> usize {
    if diff {
        (q + if bit { 1 } else { 3 }) % 4
    } else if q.is_multiple_of(2) {
        if bit { 1 } else { 3 }
    } else if bit {
        0
    } else {
        2
    }
}
pub(super) fn msk(e: &mut Engine, a: &[Value], demod: bool, nout: usize) -> Result<Vec<Value>> {
    let name = if demod { "mskdemod" } else { "mskmod" };
    arity(a, 2, if demod { 5 } else { 4 }, name)?;
    let x = numeric(e, &a[0], name, 2)?;
    let (nr, nc) = lanes(&x);
    let ns = integer(e, &a[1], 1, MAX_SAMPLES, name)?;
    let diff = choice(e, a.get(2), "diff", &["diff", "nondiff"], name)? == "diff";
    reserve(e, nc.saturating_mul(2))?;
    let mut phase = phases(e, a.get(3), nc)?;
    if demod && nr % ns != 0 {
        return Err("mskdemod: signal length must be a multiple of nsamp".into());
    }
    if !demod {
        symbols(e, &x, 2, name)?;
    }
    let state = if let Some(v) = a.get(4).filter(|v| !empty(v)) {
        let s = numeric(e, v, "mskdemod initstate", 2)?;
        if s.rows() != ns || s.cols() != nc {
            return Err("mskdemod: initstate must be nsamp-by-channels".into());
        }
        Some(s)
    } else {
        None
    };
    let rows = if demod {
        nr / ns
    } else {
        nr.checked_mul(ns).ok_or("msk: output size overflow")?
    };
    let len = rows
        .checked_mul(nc)
        .filter(|n| *n <= MAX_SAMPLES)
        .ok_or("msk: output sample limit exceeded")?;
    reserve(
        e,
        len.saturating_mul(2)
            .saturating_add(if demod && nout > 2 {
                ns.saturating_mul(nc).saturating_mul(2)
            } else {
                0
            })
            .saturating_add(nc * 2),
    )?;
    work(
        e,
        if demod {
            x.len().saturating_mul(2)
        } else {
            len
        },
    )?;
    let mut result = Vec::new();
    if demod {
        let mut y = zeros::<f64>(e, len)?;
        let mut last = zeros::<Complex64>(e, if nout > 2 { ns * nc } else { 0 })?;
        for c in 0..nc {
            let mut boundary = state
                .as_ref()
                .map(|s| at(s, c * ns + ns - 1))
                .filter(|z| z.re != 0. || z.im != 0.)
                .map_or_else(|| unit(phase[c] as f64 * FRAC_PI_2), |z| unit(z.arg()));
            for s in 0..rows {
                e.check_cancel()?;
                let scale = symbol_scale(e, &x, c * nr + s * ns, ns)?.max(f64::MIN_POSITIVE);
                let mut plus = 0.;
                let mut minus = 0.;
                for t in 0..ns {
                    if t % 4096 == 0 {
                        e.check_cancel()?;
                    }
                    let z = (at(&x, c * nr + s * ns + t) / scale) * boundary.conj();
                    let pulse = unit(FRAC_PI_2 * (t + 1) as f64 / ns as f64);
                    plus += (z * pulse.conj()).re;
                    minus += (z * pulse).re;
                }
                let up = plus >= minus;
                phase[c] = (phase[c] + if up { 1 } else { 3 }) % 4;
                y[c * rows + s] = f64::from(if diff { up } else { phase[c] < 2 });
                let z = at(&x, c * nr + (s + 1) * ns - 1);
                boundary = if z.re != 0. || z.im != 0. {
                    unit(z.arg())
                } else {
                    unit(phase[c] as f64 * FRAC_PI_2)
                };
            }
            for t in 0..if nout > 2 { ns } else { 0 } {
                if t % 4096 == 0 {
                    e.check_cancel()?;
                }
                last[c * ns + t] = if nr > 0 {
                    at(&x, c * nr + nr - ns + t)
                } else {
                    state
                        .as_ref()
                        .map_or(Complex64::new(0., 0.), |s| at(s, c * ns + t))
                };
            }
        }
        result.push(rv(rows, nc, y)?);
        if nout > 1 {
            result.push(rv(
                1,
                nc,
                phase.iter().map(|q| *q as f64 * FRAC_PI_2).collect(),
            )?);
        }
        if nout > 2 {
            result.push(cv(e, &[ns, nc], last)?);
        }
    } else {
        let mut y = zeros::<Complex64>(e, len)?;
        for c in 0..nc {
            for s in 0..nr {
                e.check_cancel()?;
                let q = phase[c];
                let next = msk_next(q, at(&x, c * nr + s).re != 0., diff);
                let direction = if (next + 4 - q) % 4 == 1 { 1. } else { -1. };
                for t in 0..ns {
                    if t % 4096 == 0 {
                        e.check_cancel()?;
                    }
                    y[c * rows + s * ns + t] =
                        unit((q as f64 + direction * (t + 1) as f64 / ns as f64) * FRAC_PI_2);
                }
                phase[c] = next;
            }
        }
        result.push(cv(e, &[rows, nc], y)?);
        if nout > 1 {
            result.push(rv(
                1,
                nc,
                phase.iter().map(|q| *q as f64 * FRAC_PI_2).collect(),
            )?);
        }
    }
    Ok(result)
}

// Shared views only; no copied symbol buffer. Relative scaling is safe for
// every finite input, including subnormal components.
fn symbol_scale(e: &Engine, x: &Array, start: usize, n: usize) -> Result<f64> {
    let mut scale = 0_f64;
    for i in 0..n {
        if i % 4096 == 0 {
            e.check_cancel()?;
        }
        let z = at(x, start + i);
        scale = scale.max(z.re.abs()).max(z.im.abs());
    }
    Ok(scale)
}
