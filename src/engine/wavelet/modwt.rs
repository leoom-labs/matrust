//! Maximal overlap discrete wavelet transform (Percival & Walden 2000, ch. 5):
//! modwt, imodwt and modwtmra with periodic or 'reflection' boundaries.
//! The MODWT filters are the orthogonal reconstruction filters of `wfilters`
//! divided by sqrt(2): scaling g = LO_R/sqrt(2), wavelet h = HI_R/sqrt(2).
//! Level j filters circularly with taps 2^(j-1) apart:
//!   W_j(t) = sum_l h(l) V_{j-1}(t - 2^(j-1) l mod N), V_j likewise with g.
use super::super::poly_interp::real_matrix;
use super::super::{Engine, Value};
use super::filters;
use super::mra::{mra_bool, mra_fft, mra_matrix, mra_scalar, mra_signal, mra_text};
use crate::Result;
use rustfft::num_complex::Complex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Tap updates between cancellation polls. The inner loop does not call out;
/// one poll covers about this many multiply-adds.
const CANCEL_EVERY: usize = 1_000_000;

struct Filters {
    g: Vec<f64>,
    h: Vec<f64>,
}

/// Wavelet name or Lo, Hi pair at args[i]; returns the filters and the
/// number of arguments used (0 when args[i] is not a wavelet).
fn modwt_filters(args: &[Value], i: usize, who: &str) -> Result<(Filters, usize)> {
    let scale = |v: &[f64]| {
        v.iter()
            .map(|c| c / std::f64::consts::SQRT_2)
            .collect::<Vec<_>>()
    };
    match args.get(i) {
        Some(v) if mra_text(v).is_some_and(|t| !is_keyword(t)) => {
            let name = mra_text(v).unwrap_or_default();
            let lower = name.trim().to_ascii_lowercase();
            if lower.starts_with("bior") || lower.starts_with("rbio") {
                return Err(format!(
                    "{who}: the wavelet must be orthogonal ('{name}' is biorthogonal)"
                ));
            }
            let b = filters::bank(name)?;
            Ok((
                Filters {
                    g: scale(&b.lo_r),
                    h: scale(&b.hi_r),
                },
                1,
            ))
        }
        Some(v) if mra_text(v).is_none() && real_matrix(v, who).is_ok_and(|m| m.len() > 1) => {
            let lo = filter_vector(v, who, "Lo")?;
            let hi = match args.get(i + 1) {
                Some(h) if mra_text(h).is_none() => filter_vector(h, who, "Hi")?,
                _ => return Err(format!("{who}: give both the Lo and Hi filters")),
            };
            if lo.len() != hi.len() || lo.len() % 2 == 1 {
                return Err(format!(
                    "{who}: Lo and Hi must be even-length filters of equal length"
                ));
            }
            Ok((
                Filters {
                    g: scale(&lo),
                    h: scale(&hi),
                },
                2,
            ))
        }
        _ => {
            let b = filters::bank("sym4")?;
            Ok((
                Filters {
                    g: scale(&b.lo_r),
                    h: scale(&b.hi_r),
                },
                0,
            ))
        }
    }
}

fn filter_vector(v: &Value, who: &str, what: &str) -> Result<Vec<f64>> {
    let m = real_matrix(v, who)?;
    if m.is_nd() || (m.rows() > 1 && m.cols() > 1) || m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("{who}: {what} must be a real finite vector"));
    }
    Ok(m.data().to_vec())
}

fn is_keyword(t: &str) -> bool {
    ["reflection", "periodic", "timealign"]
        .iter()
        .any(|k| t.eq_ignore_ascii_case(k))
}

/// Trailing 'reflection' / 'periodic' and TimeAlign=false. Returns reflection.
fn boundary(rest: &[Value], who: &str, time_align: bool) -> Result<bool> {
    let mut reflection = false;
    let mut i = 0;
    while i < rest.len() {
        let Some(t) = mra_text(&rest[i]) else {
            return Err(format!("{who}: too many input arguments"));
        };
        if t.eq_ignore_ascii_case("reflection") {
            reflection = true;
            i += 1;
        } else if t.eq_ignore_ascii_case("periodic") {
            reflection = false;
            i += 1;
        } else if time_align && t.eq_ignore_ascii_case("timealign") {
            let v = rest
                .get(i + 1)
                .ok_or_else(|| format!("{who}: 'TimeAlign' needs a value"))?;
            if mra_bool(v, who, "TimeAlign")? {
                return Err(format!("{who}: TimeAlign=true is not supported yet"));
            }
            i += 2;
        } else {
            return Err(format!(
                "{who}: unknown option '{t}' (use 'reflection' or 'periodic')"
            ));
        }
    }
    Ok(reflection)
}

fn optional_level(args: &[Value], i: usize, who: &str) -> Result<(Option<f64>, usize)> {
    match args.get(i) {
        Some(v) if mra_text(v).is_none() => Ok((Some(mra_scalar(v, who, "the level")?), 1)),
        _ => Ok((None, 0)),
    }
}

/// `floor(log2(n))` for `n >= 1`. This is modwt's maximum level.
fn floor_log2(n: usize) -> usize {
    usize::BITS as usize - 1 - n.leading_zeros() as usize
}

/// Reject a coefficient depth the signal length cannot express. The same
/// ceiling as `modwt`'s own level check: `floor(log2(N))`.
fn level_ok(levels: usize, n: usize, who: &str) -> Result<()> {
    let top = if n == 0 { 0 } else { floor_log2(n) };
    if levels >= 1 && levels <= top {
        Ok(())
    } else {
        Err(format!(
            "{who}: the level must be a positive integer no larger than floor(log2(N)) = {top}"
        ))
    }
}

/// `2^(level-1)`, or an error when the shift does not fit in `usize`.
fn level_stride(level: usize, who: &str) -> Result<usize> {
    let shift = level.checked_sub(1).and_then(|s| u32::try_from(s).ok());
    shift.and_then(|s| 1usize.checked_shl(s)).ok_or_else(|| {
        format!("{who}: the level must be a positive integer no larger than floor(log2(N))")
    })
}

/// `(s * l) mod n`. The product is widened when it does not fit in `usize`.
fn tap_offset(s: usize, l: usize, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    match s.checked_mul(l) {
        Some(p) => p % n,
        None => {
            let p = (s % n) as u128 * (l % n) as u128;
            (p % n as u128) as usize
        }
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

/// One analysis level with taps `s` samples apart.
fn analysis_step(
    v: &[f64],
    f: &Filters,
    s: usize,
    cancel: &AtomicBool,
) -> Result<(Vec<f64>, Vec<f64>)> {
    if f.h.len() > CANCEL_EVERY {
        analysis_inner::<true>(v, f, s, cancel)
    } else {
        analysis_inner::<false>(v, f, s, cancel)
    }
}

fn analysis_inner<const INNER: bool>(
    v: &[f64],
    f: &Filters,
    s: usize,
    cancel: &AtomicBool,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n = v.len();
    let mut w = vec![0.; n];
    let mut a = vec![0.; n];
    let taps = f.h.len().max(1);
    let every = (CANCEL_EVERY / taps).max(1);
    let mut left = every;
    for t in 0..n {
        if !INNER && left == 0 {
            cancelled(cancel)?;
            left = every;
        }
        if !INNER {
            left -= 1;
        }
        let (mut sw, mut sa) = (0., 0.);
        for (l, (&h, &g)) in f.h.iter().zip(&f.g).enumerate() {
            if INNER && (l + 1) % CANCEL_EVERY == 0 {
                cancelled(cancel)?;
            }
            let k = tap_offset(s, l, n);
            let idx = if t >= k { t - k } else { t + n - k };
            sw += h * v[idx];
            sa += g * v[idx];
        }
        w[t] = sw;
        a[t] = sa;
    }
    Ok((w, a))
}

/// Inverse of `analysis_step`: V_{j-1}(t) = sum_l h(l) W(t + s l) + g(l) V(t + s l).
fn synthesis_step(
    w: &[f64],
    v: &[f64],
    f: &Filters,
    s: usize,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    if f.h.len() > CANCEL_EVERY {
        synthesis_inner::<true>(w, v, f, s, cancel)
    } else {
        synthesis_inner::<false>(w, v, f, s, cancel)
    }
}

fn synthesis_inner<const INNER: bool>(
    w: &[f64],
    v: &[f64],
    f: &Filters,
    s: usize,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    let n = v.len();
    let taps = f.h.len().max(1);
    let every = (CANCEL_EVERY / taps).max(1);
    let mut left = every;
    let mut out = Vec::with_capacity(n);
    for t in 0..n {
        if !INNER && left == 0 {
            cancelled(cancel)?;
            left = every;
        }
        if !INNER {
            left -= 1;
        }
        let mut acc = 0.;
        for (l, (&h, &g)) in f.h.iter().zip(&f.g).enumerate() {
            if INNER && (l + 1) % CANCEL_EVERY == 0 {
                cancelled(cancel)?;
            }
            let idx = (t + tap_offset(s, l, n)) % n;
            acc += h * w[idx] + g * v[idx];
        }
        out.push(acc);
    }
    Ok(out)
}

/// Rows W_1..W_J then V_J.
fn forward(e: &Engine, x: &[f64], f: &Filters, levels: usize) -> Result<Vec<Vec<f64>>> {
    // (levels + 1) by N coefficients, before any per-level buffer.
    super::mra::mra_admit(e, "modwt", levels + 1, x.len())?;
    level_ok(levels, x.len(), "modwt")?;
    let mut rows = Vec::with_capacity(levels + 1);
    let mut v = x.to_vec();
    for j in 1..=levels {
        e.check_cancel()?;
        let s = level_stride(j, "modwt")?;
        let (w, a) = analysis_step(&v, f, s, &e.cancel)?;
        rows.push(w);
        v = a;
    }
    rows.push(v);
    Ok(rows)
}

/// The (L+1)-by-M coefficient matrix as rows; `who` names errors.
fn coefficient_rows(v: &Value, who: &str) -> Result<Vec<Vec<f64>>> {
    let m = real_matrix(v, who)?;
    if m.is_nd() || m.rows() < 2 || m.cols() < 2 {
        return Err(format!(
            "{who}: W must be a MODWT coefficient matrix with at least 2 rows and 2 columns"
        ));
    }
    if m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("{who}: W must contain finite values"));
    }
    let (r, c) = (m.rows(), m.cols());
    Ok((0..r)
        .map(|i| (0..c).map(|j| m.data()[i + j * r]).collect())
        .collect())
}

fn rows_value(rows: &[Vec<f64>], cols: usize) -> Result<Value> {
    let r = rows.len();
    let len = crate::Matrix::checked_len(r, cols)?;
    let mut data = vec![0.; len];
    for (i, row) in rows.iter().enumerate() {
        for j in 0..cols {
            data[i + j * r] = row[j];
        }
    }
    mra_matrix(r, cols, data)
}

pub(super) fn modwt_builtin(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    const WHO: &str = "modwt";
    let Some(first) = args.first() else {
        return Err("modwt: not enough input arguments".into());
    };
    let x = mra_signal(first, WHO, 2)?;
    let n = x.len();
    let top = floor_log2(n); // floor(log2(n))
    let mut i = 1;
    // modwt(x, lev): a numeric scalar second argument is the level.
    let (lev, used) = optional_level_scalar(args, i, WHO)?;
    let (f, fused) = if used == 1 {
        let b = filters::bank("sym4")?;
        let s = std::f64::consts::SQRT_2;
        (
            Filters {
                g: b.lo_r.iter().map(|c| c / s).collect(),
                h: b.hi_r.iter().map(|c| c / s).collect(),
            },
            0,
        )
    } else {
        modwt_filters(args, i, WHO)?
    };
    i += used + fused;
    let lev = match lev {
        Some(l) => Some(l),
        None => {
            let (l, u) = optional_level(args, i, WHO)?;
            i += u;
            l
        }
    };
    let reflection = boundary(&args[i..], WHO, true)?;
    let levels = match lev {
        None => top,
        Some(l) if l >= 1. && l.fract() == 0. && l <= top as f64 => l as usize,
        Some(_) => {
            return Err(format!(
                "modwt: the level must be a positive integer no larger than floor(log2(N)) = {top}"
            ));
        }
    };
    // Reflection stores 2N columns. Refuse that shape before the extension exists.
    let cols = if reflection {
        n.checked_mul(2).ok_or("Matrix dimensions overflow")?
    } else {
        n
    };
    super::mra::mra_admit(e, WHO, levels + 1, cols)?;
    let signal = if reflection {
        let mut ext = x.clone();
        ext.extend(x.iter().rev());
        ext
    } else {
        x
    };
    let rows = forward(e, &signal, &f, levels)?;
    Ok(vec![rows_value(&rows, signal.len())?])
}

/// A numeric scalar at args[i] (the level in modwt(x, lev, ...)).
fn optional_level_scalar(args: &[Value], i: usize, who: &str) -> Result<(Option<f64>, usize)> {
    match args.get(i) {
        Some(v) if mra_text(v).is_none() && real_matrix(v, who).is_ok_and(|m| m.len() == 1) => {
            Ok((Some(mra_scalar(v, who, "the level")?), 1))
        }
        _ => Ok((None, 0)),
    }
}

pub(super) fn imodwt_builtin(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    const WHO: &str = "imodwt";
    let Some(first) = args.first() else {
        return Err("imodwt: not enough input arguments".into());
    };
    let rows = coefficient_rows(first, WHO)?;
    let levels = rows.len() - 1;
    let mut i = 1;
    let (lev, used) = optional_level_scalar(args, i, WHO)?;
    let (f, fused) = if used == 1 {
        modwt_filters(&[], 0, WHO)?
    } else {
        modwt_filters(args, i, WHO)?
    };
    i += used + fused;
    let lev = match lev {
        Some(l) => Some(l),
        None => {
            let (l, u) = optional_level(args, i, WHO)?;
            i += u;
            l
        }
    };
    let reflection = boundary(&args[i..], WHO, false)?;
    let stop = match lev {
        None => 0,
        Some(l) if l >= 0. && l.fract() == 0. && l < levels as f64 => l as usize,
        Some(_) => {
            return Err(format!(
                "imodwt: the reconstruction level must be an integer from 0 to {}",
                levels - 1
            ));
        }
    };
    let m = rows[0].len();
    level_ok(levels, m, WHO)?;
    if reflection && m % 2 == 1 {
        return Err("imodwt: 'reflection' needs an even number of columns in W".into());
    }
    let mut v = rows[levels].clone();
    for j in (stop + 1..=levels).rev() {
        e.check_cancel()?;
        let s = level_stride(j, WHO)?;
        v = synthesis_step(&rows[j - 1], &v, &f, s, &e.cancel)?;
    }
    if reflection {
        v.truncate(m / 2);
    }
    let len = v.len();
    Ok(vec![mra_matrix(1, len, v)?])
}

pub(super) fn modwtmra_builtin(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    const WHO: &str = "modwtmra";
    let Some(first) = args.first() else {
        return Err("modwtmra: not enough input arguments".into());
    };
    let rows = coefficient_rows(first, WHO)?;
    let (f, used) = modwt_filters(args, 1, WHO)?;
    let reflection = boundary(&args[1 + used..], WHO, false)?;
    let m = rows[0].len();
    level_ok(rows.len() - 1, m, WHO)?;
    if reflection && m % 2 == 1 {
        return Err("modwtmra: 'reflection' needs an even number of columns in W".into());
    }
    let mut mra = modwt_mra(e, &rows, &f)?;
    let keep = if reflection { m / 2 } else { m };
    for r in &mut mra {
        r.truncate(keep);
    }
    Ok(vec![rows_value(&mra, keep)?])
}

/// DFT of a filter with taps `s` apart, wrapped onto n points.
fn dilated_response(c: &[f64], s: usize, n: usize) -> Vec<Complex<f64>> {
    let mut buf = vec![Complex::new(0., 0.); n];
    if n == 0 {
        return buf;
    }
    for (l, &v) in c.iter().enumerate() {
        buf[tap_offset(s, l, n)].re += v;
    }
    mra_fft(&mut buf, false);
    buf
}

/// Details D_1..D_J and smooth S_J: each level's coefficients pushed back
/// through the synthesis filters alone (in the frequency domain).
fn modwt_mra(e: &Engine, rows: &[Vec<f64>], f: &Filters) -> Result<Vec<Vec<f64>>> {
    let levels = rows.len() - 1;
    let n = rows[0].len();
    // One real row per coefficient row, plus the complex frequency scratch.
    super::mra::mra_admit(e, "modwtmra", rows.len(), n)?;
    super::mra::mra_admit_complex(e, "modwtmra", n, 1)?;
    let mut cum = vec![Complex::new(1., 0.); n]; // prod_{i<j} G_i
    let mut out = Vec::with_capacity(levels + 1);
    let inv_n = 1. / n as f64;
    let back = |row: &[f64], resp: &[Complex<f64>]| -> Vec<f64> {
        let mut buf: Vec<Complex<f64>> = row.iter().map(|&v| Complex::new(v, 0.)).collect();
        mra_fft(&mut buf, false);
        for (b, r) in buf.iter_mut().zip(resp) {
            *b *= r.conj();
        }
        mra_fft(&mut buf, true);
        buf.iter().map(|c| c.re * inv_n).collect()
    };
    for j in 1..=levels {
        e.check_cancel()?;
        let s = level_stride(j, "modwtmra")?;
        let hj = dilated_response(&f.h, s, n);
        let gj = dilated_response(&f.g, s, n);
        let resp: Vec<Complex<f64>> = hj.iter().zip(&cum).map(|(a, b)| a * b).collect();
        out.push(back(&rows[j - 1], &resp));
        for (c, g) in cum.iter_mut().zip(&gj) {
            *c *= g;
        }
    }
    out.push(back(&rows[levels], &cum));
    Ok(out)
}
