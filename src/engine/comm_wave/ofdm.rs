//! Centered subcarriers -> IFFT -> cyclic prefix. One reusable RustFFT plan and
//! scratch lane per call; no retained plans, transpose copies or thread pools.
use super::*;
use rustfft::FftPlanner;
fn indices(e: &Engine, v: Option<&Value>, n: usize, name: &str) -> Result<Vec<usize>> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let m = vector(e, v, name)?;
    if m.len() > n {
        return Err(format!("{name}: too many subcarrier indices"));
    }
    reserve(e, n.saturating_add(m.len()))?;
    let mut seen = zeros::<bool>(e, n)?;
    let mut ids = Vec::with_capacity(m.len());
    for &x in m.data() {
        if x.fract() != 0. || x < 1. || x > n as f64 {
            return Err(format!(
                "{name}: subcarrier indices must be integers in 1..nfft"
            ));
        }
        let k = x as usize - 1;
        if seen[k] {
            return Err(format!("{name}: duplicate subcarrier index"));
        }
        seen[k] = true;
        ids.push(k);
    }
    Ok(ids)
}
fn scaled(x: f64, os: f64, name: &str) -> Result<usize> {
    let v = x * os;
    if !v.is_finite() || v < 0. || v.fract() != 0. || v > MAX_SAMPLES as f64 {
        return Err(format!(
            "{name}: scaled lengths must be integers within the sample limit"
        ));
    }
    Ok(v as usize)
}
pub(super) fn call(e: &mut Engine, a: &[Value], demod: bool, nout: usize) -> Result<Vec<Value>> {
    let name = if demod { "ofdmdemod" } else { "ofdmmod" };
    arity(a, 3, 8, name)?;
    let split = a
        .iter()
        .enumerate()
        .skip(3)
        .find(|(_, v)| super::super::table_fns::text_scalar(v).is_some())
        .map_or(a.len(), |(i, _)| i);
    let pos = &a[..split];
    if (demod && !(3..=6).contains(&pos.len())) || (!demod && ![3, 4, 6].contains(&pos.len())) {
        return Err(format!("{name}: invalid positional arguments"));
    }
    let os = if split < a.len() {
        if a.len() != split + 2
            || !text(e, &a[split], name)?.eq_ignore_ascii_case("OversamplingFactor")
        {
            return Err(format!(
                "{name}: expected OversamplingFactor name-value pair"
            ));
        }
        positive(e, &a[split + 1], name)?
    } else {
        1.
    };
    if os < 1. {
        return Err(format!(
            "{name}: OversamplingFactor below 1 is not supported"
        ));
    }
    let x = numeric(e, &a[0], name, if demod { 3 } else { 4 })?;
    let n = integer(e, &a[1], 8, crate::dsp::MAX_FFT_LENGTH, name)?;
    let nf = scaled(n as f64, os, name)?;
    if nf > crate::dsp::MAX_FFT_LENGTH {
        return Err(format!("{name}: oversampled FFT exceeds 1048576 points"));
    }
    let cp = vector(e, &a[2], name)?;
    if cp.is_empty()
        || cp.rows() != 1
        || cp.data().iter().any(|v| *v < 0. || v.fract() != 0.)
        || (cp.len() == 1 && cp.data()[0] > n as f64)
    {
        return Err(format!(
            "{name}: cplen must be a nonnegative integer scalar (<=nfft) or row vector"
        ));
    }
    reserve(
        e,
        cp.len()
            .saturating_mul(4)
            .saturating_add(n.saturating_mul(4)),
    )?;
    let cp = cp
        .data()
        .iter()
        .map(|v| scaled(*v, os, name))
        .collect::<Result<Vec<_>>>()?;
    let null = indices(e, pos.get(if demod { 4 } else { 3 }), n, name)?;
    let pilot = indices(e, pos.get(if demod { 5 } else { 4 }), n, name)?;
    let mut occupied = zeros::<bool>(e, n)?;
    for &i in &null {
        occupied[i] = true;
    }
    for &i in &pilot {
        if occupied[i] {
            return Err(format!("{name}: null and pilot indices must be disjoint"));
        }
        occupied[i] = true;
    }
    let data = (0..n).filter(|i| !occupied[*i]).collect::<Vec<_>>();
    let dims = x.dims();
    let (ns, nc, nb, outrows) = if demod {
        let ns = if cp.len() == 1 {
            if x.rows() % (nf + cp[0]) != 0 {
                return Err("ofdmdemod: input length does not match nfft and cplen".into());
            }
            x.rows() / (nf + cp[0])
        } else {
            if nf
                .saturating_mul(cp.len())
                .saturating_add(cp.iter().sum::<usize>())
                != x.rows()
            {
                return Err("ofdmdemod: input length does not match cplen vector".into());
            }
            cp.len()
        };
        (ns, dims[1], *dims.get(2).unwrap_or(&1), x.rows())
    } else {
        if dims[0] != data.len() {
            return Err("ofdmmod: data rows must equal nfft minus null and pilot counts".into());
        }
        let ns = dims[1];
        if cp.len() != 1 && cp.len() != ns {
            return Err("ofdmmod: cplen vector must have one value per symbol".into());
        }
        let rows = nf.saturating_mul(ns).saturating_add(if cp.len() == 1 {
            cp[0].saturating_mul(ns)
        } else {
            cp.iter().sum()
        });
        (
            ns,
            *dims.get(2).unwrap_or(&1),
            *dims.get(3).unwrap_or(&1),
            rows,
        )
    };
    let p = if !demod && pos.len() == 6 {
        let p = numeric(e, &pos[5], "ofdmmod pilots", 4)?;
        let mut pd = p.dims().into_owned();
        pd.resize(4, 1);
        if pd != [pilot.len(), ns, nc, nb] {
            return Err(
                "ofdmmod: pilots must be numPilots-by-symbols-by-streams-by-batches".into(),
            );
        }
        Some(p)
    } else {
        None
    };
    let offsets = if demod {
        match pos.get(3).filter(|v| !empty(v)) {
            Some(v) => {
                let m = vector(e, v, name)?;
                if m.rows() != 1 || (m.len() != 1 && m.len() != ns) {
                    return Err(
                        "ofdmdemod: symOffset must be scalar or one value per symbol".into(),
                    );
                }
                m.data()
                    .iter()
                    .map(|v| scaled(*v, os, name))
                    .collect::<Result<Vec<_>>>()?
            }
            None => cp.clone(),
        }
    } else {
        Vec::new()
    };
    if demod
        && (0..ns).any(|s| {
            offsets[if offsets.len() == 1 { 0 } else { s }] > cp[if cp.len() == 1 { 0 } else { s }]
        })
    {
        return Err("ofdmdemod: symOffset must lie in [0,cplen]".into());
    }
    let lanes = nc.checked_mul(nb).ok_or("ofdm: stream count overflow")?;
    let len = if demod {
        n.saturating_mul(ns).saturating_mul(lanes)
    } else {
        outrows.saturating_mul(lanes)
    };
    if len > MAX_SAMPLES {
        return Err("ofdm: output sample limit exceeded".into());
    }
    // Conservative plan allowance also bounds Bluestein's construction buffers.
    reserve(
        e,
        len.saturating_mul(2)
            .saturating_add(nf.saturating_mul(66))
            .saturating_add(n.saturating_mul(4)),
    )?;
    work(
        e,
        nf.saturating_mul(ns)
            .saturating_mul(lanes)
            .saturating_mul((nf as f64).log2().ceil() as usize),
    )?;
    let mut planner = FftPlanner::<f64>::new();
    let fft = if demod {
        planner.plan_fft_forward(nf)
    } else {
        planner.plan_fft_inverse(nf)
    };
    drop(planner);
    e.check_cancel()?;
    let scratchlen = fft.get_inplace_scratch_len();
    reserve(
        e,
        len.saturating_mul(2)
            .saturating_add((nf + scratchlen).saturating_mul(2))
            .saturating_add(nf.saturating_mul(64)),
    )?;
    let mut scratch = zeros::<Complex64>(e, scratchlen)?;
    let mut buf = zeros::<Complex64>(e, nf)?;
    let mut output = zeros::<Complex64>(e, if demod { data.len() * ns * lanes } else { len })?;
    let mut pilots = zeros::<Complex64>(
        e,
        if demod && nout > 1 {
            pilot.len() * ns * lanes
        } else {
            0
        },
    )?;
    // Map centered frequencies, including odd FFT sizes, into raw FFT bins.
    let bin = |k: usize| ((k as isize - n as isize / 2).rem_euclid(nf as isize)) as usize;
    for lane in 0..lanes {
        let mut start = 0;
        for s in 0..ns {
            e.check_cancel()?;
            let prefix = cp[if cp.len() == 1 { 0 } else { s }];
            if demod {
                let offset = offsets[if offsets.len() == 1 { 0 } else { s }];
                for (t, v) in buf.iter_mut().enumerate() {
                    if t % 4096 == 0 {
                        e.check_cancel()?;
                    }
                    *v = at(&x, lane * outrows + start + offset + t);
                }
                fft.process_with_scratch(&mut buf, &mut scratch);
                let shift = prefix - offset;
                let get = |k: usize| {
                    buf[bin(k)] * unit(TAU * bin(k) as f64 * shift as f64 / nf as f64) / os
                };
                for (j, &k) in data.iter().enumerate() {
                    output[(lane * ns + s) * data.len() + j] = get(k);
                }
                if nout > 1 {
                    for (j, &k) in pilot.iter().enumerate() {
                        pilots[(lane * ns + s) * pilot.len() + j] = get(k);
                    }
                }
            } else {
                buf.fill(Complex64::new(0., 0.));
                for (j, &k) in data.iter().enumerate() {
                    buf[bin(k)] = at(&x, (lane * ns + s) * data.len() + j);
                }
                if let Some(p) = &p {
                    for (j, &k) in pilot.iter().enumerate() {
                        buf[bin(k)] = at(p, (lane * ns + s) * pilot.len() + j);
                    }
                }
                fft.process_with_scratch(&mut buf, &mut scratch);
                for t in 0..prefix + nf {
                    if t % 4096 == 0 {
                        e.check_cancel()?;
                    }
                    let source = (t + nf - prefix % nf) % nf;
                    output[lane * outrows + start + t] = buf[source] / n as f64;
                }
            }
            start += prefix + nf;
        }
    }
    let mut result = vec![cv(
        e,
        &if demod {
            vec![data.len(), ns, nc, nb]
        } else {
            vec![outrows, nc, nb]
        },
        output,
    )?];
    if demod && nout > 1 {
        result.push(cv(e, &[pilot.len(), ns, nc, nb], pilots)?);
    }
    Ok(result)
}
