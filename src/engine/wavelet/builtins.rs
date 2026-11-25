//! Argument handling and outputs of the wavelet builtins. Outputs keep the
//! orientation of the input vector (a column in gives columns out).
use super::super::poly_interp::real_matrix;
use super::super::{Engine, Execution, Value};
use super::denoise::{self, Method, Rule, ThRule};
use super::filters;
use super::transform::{self, Mode};
use crate::{Array, CellArray, Matrix, Result};
use std::sync::Arc;

/// Deepest decomposition level accepted (2^64 samples would be needed to use it).
const MAX_LEVEL: usize = 64;

fn max_outputs(name: &str) -> usize {
    match name {
        "wfilters" | "wden" => 4,
        "dwt" | "wavedec" => 2,
        "wdenoise" => 3,
        "detcoef" => usize::MAX,
        _ => 1,
    }
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if nargout > max_outputs(name) {
        return Err(format!("{name}: too many output arguments"));
    }
    match name {
        "wfilters" => wfilters(args, nargout),
        "dwt" => dwt(e, args),
        "idwt" => idwt(e, args),
        "dwtmode" => dwtmode(e, args, nargout, out),
        "wavedec" => wavedec(e, args),
        "waverec" => waverec(e, args),
        "appcoef" => appcoef(e, args),
        "detcoef" => detcoef(e, args, nargout),
        "wrcoef" => wrcoef(e, args),
        "wmaxlev" => wmaxlev(args),
        "wthresh" => wthresh(args),
        "thselect" => thselect(args),
        "wden" => wden(e, args),
        "wdenoise" => wdenoise(e, args, nargout),
        _ => Err(format!("Undefined function '{name}'")),
    }
}

// ---------- argument helpers ----------

fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

struct Vector {
    data: Vec<f64>,
    column: bool,
}

fn vector(v: &Value, who: &str, what: &str, allow_empty: bool) -> Result<Vector> {
    let m = real_matrix(v, who)?;
    if m.is_nd() || (m.rows() > 1 && m.cols() > 1) {
        return Err(format!("{who}: {what} must be a vector"));
    }
    if m.is_empty() && !allow_empty {
        return Err(format!("{who}: {what} must not be empty"));
    }
    if m.data().iter().any(|v| !v.is_finite()) {
        return Err(format!("{who}: {what} must contain finite values"));
    }
    Ok(Vector {
        data: m.data().to_vec(),
        column: m.cols() == 1 && m.rows() > 1,
    })
}

fn out_vec(data: Vec<f64>, column: bool) -> Result<Value> {
    let n = data.len();
    let (r, c) = if column { (n, 1) } else { (1, n) };
    Ok(Value::Matrix(Matrix::from_column_vec(r, c, data)?))
}

fn out_scalar(v: f64) -> Value {
    Value::Matrix(Matrix::scalar(v))
}

fn integer(v: &Value, who: &str, what: &str) -> Result<usize> {
    let m = real_matrix(v, who).map_err(|_| format!("{who}: {what} must be an integer"))?;
    match m.data() {
        [x] if *x >= 0. && x.fract() == 0. && *x < 1e15 => Ok(*x as usize),
        _ => Err(format!(
            "{who}: {what} must be a nonnegative integer scalar"
        )),
    }
}

fn level_arg(v: &Value, who: &str) -> Result<usize> {
    let n = integer(v, who, "the level N")?;
    if n == 0 || n > MAX_LEVEL {
        return Err(format!(
            "{who}: the level N must be an integer between 1 and {MAX_LEVEL}"
        ));
    }
    Ok(n)
}

fn bookkeeping(v: &Value, who: &str) -> Result<Vec<usize>> {
    let l = vector(v, who, "the bookkeeping vector L", false)?;
    l.data
        .iter()
        .map(|&x| {
            if x >= 1. && x.fract() == 0. {
                Ok(x as usize)
            } else {
                Err(format!(
                    "{who}: the bookkeeping vector L must hold positive integers"
                ))
            }
        })
        .collect()
}

/// Filters from a wavelet name (one argument) or a lowpass/highpass pair
/// (two arguments); `analysis` picks LO_D/HI_D over LO_R/HI_R.
fn filter_pair(
    args: &[Value],
    i: usize,
    who: &str,
    analysis: bool,
) -> Result<(Vec<f64>, Vec<f64>, usize)> {
    let Some(first) = args.get(i) else {
        return Err(format!("{who}: not enough input arguments (wavelet name)"));
    };
    if let Some(name) = text(first) {
        let b = filters::bank(name)?;
        return Ok(if analysis {
            (b.lo_d, b.hi_d, 1)
        } else {
            (b.lo_r, b.hi_r, 1)
        });
    }
    let Some(second) = args.get(i + 1) else {
        return Err(format!(
            "{who}: give a wavelet name or both lowpass and highpass filters"
        ));
    };
    let lo = vector(first, who, "the lowpass filter", false)?.data;
    let hi = vector(second, who, "the highpass filter", false)?.data;
    if lo.len() != hi.len() || lo.len() < 2 {
        return Err(format!(
            "{who}: the lowpass and highpass filters must have the same length (at least 2)"
        ));
    }
    Ok((lo, hi, 2))
}

/// Trailing 'mode', MODE pairs.
fn mode_option(e: &Engine, rest: &[Value], who: &str) -> Result<Mode> {
    let mut mode = e.wavelet.mode;
    let mut i = 0;
    while i < rest.len() {
        match text(&rest[i]) {
            Some(n) if n.eq_ignore_ascii_case("mode") => {
                let value = rest.get(i + 1).and_then(text).ok_or_else(|| {
                    format!("{who}: 'mode' must be followed by an extension mode name")
                })?;
                mode = Mode::parse(value)
                    .ok_or_else(|| format!("{who}: invalid extension mode '{value}'"))?;
                i += 2;
            }
            Some(n) => return Err(format!("{who}: unknown option '{n}'")),
            None => return Err(format!("{who}: too many input arguments")),
        }
    }
    Ok(mode)
}

fn need(args: &[Value], n: usize, who: &str) -> Result<()> {
    if args.len() < n {
        Err(format!("{who}: not enough input arguments"))
    } else {
        Ok(())
    }
}

fn cell_row(e: &Engine, items: Vec<Value>) -> Result<Value> {
    let slots = items
        .into_iter()
        .map(|v| match v {
            Value::Matrix(m) => Ok(Arc::new(Array::Numeric(m))),
            _ => Err("wavelet: internal cell element".to_string()),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        1,
        slots.len(),
        slots,
        &e.cancel,
    )?)))
}

// ---------- builtins ----------

fn wfilters(args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    need(args, 1, "wfilters")?;
    if args.len() > 2 {
        return Err("wfilters: too many input arguments".into());
    }
    let name = text(&args[0]).ok_or("wfilters: the wavelet name must be a character vector")?;
    let b = filters::bank(name)?;
    let row = |v: Vec<f64>| out_vec(v, false);
    let pair = |x: Vec<f64>, y: Vec<f64>| -> Result<Vec<Value>> { Ok(vec![row(x)?, row(y)?]) };
    match args
        .get(1)
        .map(|v| text(v).ok_or("wfilters: TYPE must be 'd', 'r', 'l' or 'h'"))
    {
        None => {
            let _ = nargout;
            Ok(vec![row(b.lo_d)?, row(b.hi_d)?, row(b.lo_r)?, row(b.hi_r)?])
        }
        Some(t) => {
            let t = t?;
            let v = match t {
                "d" => pair(b.lo_d, b.hi_d)?,
                "r" => pair(b.lo_r, b.hi_r)?,
                "l" => pair(b.lo_d, b.lo_r)?,
                "h" => pair(b.hi_d, b.hi_r)?,
                _ => return Err("wfilters: TYPE must be 'd', 'r', 'l' or 'h'".into()),
            };
            if nargout > 2 {
                return Err("wfilters: too many output arguments".into());
            }
            Ok(v)
        }
    }
}

fn dwt(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    need(args, 2, "dwt")?;
    let x = vector(&args[0], "dwt", "X", false)?;
    let (lo, hi, used) = filter_pair(args, 1, "dwt", true)?;
    let mode = mode_option(e, &args[1 + used..], "dwt")?;
    let (a, d) = transform::dwt(&x.data, &lo, &hi, mode);
    Ok(vec![out_vec(a, x.column)?, out_vec(d, x.column)?])
}

fn idwt(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    need(args, 3, "idwt")?;
    let a = vector(&args[0], "idwt", "cA", true)?;
    let d = vector(&args[1], "idwt", "cD", true)?;
    if a.data.is_empty() && d.data.is_empty() {
        return Err("idwt: cA and cD cannot both be empty".into());
    }
    let (lo, hi, used) = filter_pair(args, 2, "idwt", false)?;
    let mut rest = &args[2 + used..];
    let mut keep = None;
    if let Some(v) = rest.first()
        && text(v).is_none()
    {
        let n = integer(v, "idwt", "the length L")?;
        if n == 0 {
            return Err("idwt: the length L must be positive".into());
        }
        keep = Some(n);
        rest = &rest[1..];
    }
    let mode = mode_option(e, rest, "idwt")?;
    let column = if a.data.is_empty() {
        d.column
    } else {
        a.column
    };
    let x = transform::idwt(&a.data, &d.data, &lo, &hi, mode, keep)?;
    Ok(vec![out_vec(x, column)?])
}

fn dwtmode(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let words = args
        .iter()
        .map(|v| text(v).ok_or("dwtmode: arguments must be character vectors"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let (cmd, nodisp) = match words.as_slice() {
        [] => ("status", false),
        [c] => (*c, false),
        [c, n] if n.eq_ignore_ascii_case("nodisp") => (*c, true),
        _ => return Err("dwtmode: use dwtmode(MODE) or dwtmode('status', 'nodisp')".into()),
    };
    let banner = |m: Mode| {
        format!(
            "*******************************************\n\
             **  DWT Extension Mode: {}  **\n\
             *******************************************\n",
            m.describe()
        )
    };
    if cmd.eq_ignore_ascii_case("status") {
        if nargout == 0 && !nodisp {
            e.emit(out, banner(e.wavelet.mode))?;
            return Ok(Vec::new());
        }
        return Ok(vec![Value::Text(e.wavelet.mode.name().into())]);
    }
    if cmd.eq_ignore_ascii_case("save") {
        return Err("dwtmode: 'save' (a mode kept across sessions) is not supported".into());
    }
    let mode =
        Mode::parse(cmd).ok_or_else(|| format!("dwtmode: invalid extension mode '{cmd}'"))?;
    e.wavelet.mode = mode;
    if !nodisp && nargout == 0 {
        e.emit(
            out,
            format!("!! WARNING: Change DWT Extension Mode !!\n{}", banner(mode)),
        )?;
    }
    Ok(if nargout > 0 {
        vec![Value::Text(mode.name().into())]
    } else {
        Vec::new()
    })
}

fn wavedec(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    need(args, 3, "wavedec")?;
    let x = vector(&args[0], "wavedec", "X", false)?;
    let n = level_arg(&args[1], "wavedec")?;
    let (lo, hi, used) = filter_pair(args, 2, "wavedec", true)?;
    if args.len() > 2 + used {
        return Err("wavedec: too many input arguments".into());
    }
    let mode = e.wavelet.mode;
    let (c, l) = transform::wavedec(&x.data, n, &lo, &hi, mode, &mut || e.check_cancel())?;
    let l = l.into_iter().map(|v| v as f64).collect();
    Ok(vec![out_vec(c, x.column)?, out_vec(l, x.column)?])
}

/// C, L, then reconstruction filters; returns the rest of the arguments.
struct Decomposition {
    c: Vec<f64>,
    l: Vec<usize>,
    levels: usize,
    column: bool,
}

fn decomposition(args: &[Value], at: usize, who: &str) -> Result<Decomposition> {
    let c = vector(&args[at], who, "C", false)?;
    let l = bookkeeping(&args[at + 1], who)?;
    let levels = transform::check_book(&c.data, &l, who)?;
    Ok(Decomposition {
        c: c.data,
        l,
        levels,
        column: c.column,
    })
}

fn waverec(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    need(args, 3, "waverec")?;
    let dec = decomposition(args, 0, "waverec")?;
    let (lo, hi, used) = filter_pair(args, 2, "waverec", false)?;
    if args.len() > 2 + used {
        return Err("waverec: too many input arguments".into());
    }
    let mode = e.wavelet.mode;
    let x = transform::approx(&dec.c, &dec.l, &lo, &hi, 0, mode, &mut || e.check_cancel())?;
    Ok(vec![out_vec(x, dec.column)?])
}

fn appcoef(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    need(args, 3, "appcoef")?;
    let dec = decomposition(args, 0, "appcoef")?;
    let (lo, hi, used) = filter_pair(args, 2, "appcoef", false)?;
    let n = match args.get(2 + used) {
        Some(v) => integer(v, "appcoef", "the level N")?,
        None => dec.levels,
    };
    if args.len() > 3 + used {
        return Err("appcoef: too many input arguments".into());
    }
    if n > dec.levels {
        return Err(format!(
            "appcoef: the level N must be between 0 and {} (the decomposition depth)",
            dec.levels
        ));
    }
    let mode = e.wavelet.mode;
    let a = transform::approx(&dec.c, &dec.l, &lo, &hi, n, mode, &mut || e.check_cancel())?;
    Ok(vec![out_vec(a, dec.column)?])
}

fn detcoef(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    need(args, 2, "detcoef")?;
    let dec = decomposition(args, 0, "detcoef")?;
    let mut rest = &args[2..];
    let mut cells = false;
    if let Some(last) = rest.last().and_then(text) {
        if !last.eq_ignore_ascii_case("cells") {
            return Err(format!("detcoef: unknown option '{last}'"));
        }
        cells = true;
        rest = &rest[..rest.len() - 1];
    }
    if rest.len() > 1 {
        return Err("detcoef: too many input arguments".into());
    }
    let levels: Vec<usize> = match rest.first() {
        None if cells => (1..=dec.levels).collect(),
        None => vec![1],
        Some(v) => {
            let m = real_matrix(v, "detcoef")?;
            m.data()
                .iter()
                .map(|&k| {
                    if k >= 1. && k.fract() == 0. && k <= dec.levels as f64 {
                        Ok(k as usize)
                    } else {
                        Err(format!(
                            "detcoef: levels N must be integers between 1 and {}",
                            dec.levels
                        ))
                    }
                })
                .collect::<Result<_>>()?
        }
    };
    if levels.is_empty() {
        return Err("detcoef: the level vector N is empty".into());
    }
    let parts = levels
        .iter()
        .map(|&k| out_vec(transform::detail(&dec.c, &dec.l, k), dec.column))
        .collect::<Result<Vec<_>>>()?;
    if nargout > 1 {
        if cells || nargout != parts.len() {
            return Err("detcoef: the number of outputs must match the number of levels".into());
        }
        return Ok(parts);
    }
    if cells || parts.len() > 1 {
        return Ok(vec![cell_row(e, parts)?]);
    }
    Ok(parts)
}

fn wrcoef(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    need(args, 4, "wrcoef")?;
    let approx = match text(&args[0]).map(str::to_ascii_lowercase).as_deref() {
        Some("a") => true,
        Some("d") => false,
        _ => return Err("wrcoef: TYPE must be 'a' or 'd'".into()),
    };
    let dec = decomposition(args, 1, "wrcoef")?;
    let (lo, hi, used) = filter_pair(args, 3, "wrcoef", false)?;
    let n = match args.get(3 + used) {
        Some(v) => integer(v, "wrcoef", "the level N")?,
        None => dec.levels,
    };
    if args.len() > 4 + used {
        return Err("wrcoef: too many input arguments".into());
    }
    if n > dec.levels || (!approx && n == 0) {
        return Err(format!(
            "wrcoef: the level N must be between {} and {}",
            usize::from(!approx),
            dec.levels
        ));
    }
    let mode = e.wavelet.mode;
    let x = transform::reconstruct_branch(&dec.c, &dec.l, &lo, &hi, approx, n, mode, &mut || {
        e.check_cancel()
    })?;
    Ok(vec![out_vec(x, dec.column)?])
}

fn wmaxlev(args: &[Value]) -> Result<Vec<Value>> {
    if args.len() != 2 {
        return Err("wmaxlev: use wmaxlev(sizeX, wname)".into());
    }
    let s = vector(&args[0], "wmaxlev", "sizeX", false)?.data;
    if s.iter().any(|&v| v < 0. || v.fract() != 0.) {
        return Err("wmaxlev: sizeX must hold nonnegative integers".into());
    }
    let max = s.iter().cloned().fold(0., f64::max);
    let min = s.iter().cloned().fold(f64::INFINITY, f64::min);
    let len = if s.len() == 1 || min == 1. { max } else { min };
    let name = text(&args[1]).ok_or("wmaxlev: the wavelet name must be a character vector")?;
    let b = filters::bank(name)?;
    Ok(vec![out_scalar(
        transform::max_level(len as usize, b.lo_d.len()) as f64,
    )])
}

fn sorh(v: &Value, who: &str) -> Result<bool> {
    match text(v).map(str::to_ascii_lowercase).as_deref() {
        Some("s") => Ok(true),
        Some("h") => Ok(false),
        _ => Err(format!("{who}: SORH must be 's' (soft) or 'h' (hard)")),
    }
}

fn wthresh(args: &[Value]) -> Result<Vec<Value>> {
    if args.len() != 3 {
        return Err("wthresh: use wthresh(X, SORH, T)".into());
    }
    let m = real_matrix(&args[0], "wthresh")?;
    let soft = sorh(&args[1], "wthresh")?;
    let t = real_matrix(&args[2], "wthresh")?;
    let [t] = t.data() else {
        return Err("wthresh: the threshold T must be a scalar".into());
    };
    if t.is_nan() || *t < 0. {
        return Err("wthresh: the threshold T must be nonnegative".into());
    }
    let data = m
        .data()
        .iter()
        .map(|&v| denoise::wthresh(v, soft, *t))
        .collect();
    let r = Matrix::from_column_vec(m.rows(), m.cols(), data)?;
    let r = if m.is_nd() {
        r.with_dims(&m.dims())?
    } else {
        r
    };
    Ok(vec![Value::Matrix(r)])
}

fn rule_arg(v: &Value, who: &str) -> Result<Rule> {
    let t = text(v).ok_or_else(|| format!("{who}: TPTR must be a character vector"))?;
    if t.eq_ignore_ascii_case("modwtsqtwolog") {
        return Err(format!(
            "{who}: 'modwtsqtwolog' (MODWT denoising) is not supported yet"
        ));
    }
    Rule::parse(t).ok_or_else(|| {
        format!(
            "{who}: unknown threshold rule '{t}' (use 'rigrsure', 'heursure', 'sqtwolog' or 'minimaxi')"
        )
    })
}

fn thselect(args: &[Value]) -> Result<Vec<Value>> {
    if args.len() != 2 {
        return Err("thselect: use thselect(X, TPTR)".into());
    }
    let m = real_matrix(&args[0], "thselect")?;
    if m.data().iter().any(|v| !v.is_finite()) {
        return Err("thselect: X must contain finite values".into());
    }
    let rule = rule_arg(&args[1], "thselect")?;
    Ok(vec![out_scalar(denoise::thselect(m.data(), rule))])
}

fn wden(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    // wden(X, TPTR, SORH, SCAL, N, wname) or wden(C, L, TPTR, SORH, SCAL, N, wname)
    let (dec, at, x_column) = match args.len() {
        6 => {
            let x = vector(&args[0], "wden", "X", false)?;
            let n = level_arg(&args[4], "wden")?;
            let (lo, hi, _) = filter_pair(args, 5, "wden", true)?;
            let mode = e.wavelet.mode;
            let (c, l) = transform::wavedec(&x.data, n, &lo, &hi, mode, &mut || e.check_cancel())?;
            let dec = Decomposition {
                c,
                l,
                levels: n,
                column: x.column,
            };
            (dec, 1, x.column)
        }
        7 => {
            let dec = decomposition(args, 0, "wden")?;
            let column = dec.column;
            (dec, 2, column)
        }
        _ => {
            return Err(
                "wden: use wden(X, TPTR, SORH, SCAL, N, wname) or wden(C, L, TPTR, SORH, SCAL, N, wname)"
                    .into(),
            );
        }
    };
    let rule = rule_arg(&args[at], "wden")?;
    let soft = sorh(&args[at + 1], "wden")?;
    let scal = text(&args[at + 2])
        .map(str::to_ascii_lowercase)
        .filter(|s| matches!(s.as_str(), "one" | "sln" | "mln"))
        .ok_or("wden: SCAL must be 'one', 'sln' or 'mln'")?;
    let n = level_arg(&args[at + 3], "wden")?;
    if n > dec.levels {
        return Err(format!(
            "wden: the level N must not exceed the decomposition depth {}",
            dec.levels
        ));
    }
    let name = text(&args[at + 4]).ok_or("wden: the wavelet name must be a character vector")?;
    let bank = filters::bank(name)?;
    let details: Vec<Vec<f64>> = (1..=n)
        .map(|k| transform::detail(&dec.c, &dec.l, k))
        .collect();
    let s: Vec<f64> = match scal.as_str() {
        "one" => vec![1.; n],
        "sln" => vec![denoise::noise_sigma(&details[0]); n],
        _ => details.iter().map(|d| denoise::noise_sigma(d)).collect(),
    };
    let mut cxd = dec.c.clone();
    let mut thr = Vec::with_capacity(n);
    for (k, d) in details.iter().enumerate() {
        e.check_cancel()?;
        let unit = if matches!(rule, Rule::Sqtwolog | Rule::Minimaxi) {
            denoise::thselect(&dec.c, rule)
        } else {
            let peak = d.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if s[k] < f64::EPSILON.sqrt() * peak {
                0.
            } else {
                let scaled: Vec<f64> = d.iter().map(|v| v / s[k]).collect();
                denoise::thselect(&scaled, rule)
            }
        };
        let t = unit * s[k];
        thr.push(t);
        let start: usize = dec.l[..=dec.levels - (k + 1)].iter().sum();
        for (slot, &v) in cxd[start..start + d.len()].iter_mut().zip(d) {
            *slot = denoise::wthresh(v, soft, t);
        }
    }
    let mode = e.wavelet.mode;
    let xd = transform::approx(&cxd, &dec.l, &bank.lo_r, &bank.hi_r, 0, mode, &mut || {
        e.check_cancel()
    })?;
    let l = dec.l.iter().map(|&v| v as f64).collect();
    Ok(vec![
        out_vec(xd, x_column)?,
        out_vec(cxd, dec.column)?,
        out_vec(l, dec.column)?,
        out_vec(thr, false)?,
    ])
}

// ---------- wdenoise ----------

/// Coefficient length of one analysis step (`dwt`: periodized length, or
/// `(n + lf - 1) / 2` for every other extension).
fn analysis_len(n: usize, lf: usize, mode: Mode) -> Result<usize> {
    if mode == Mode::Per {
        return Ok(n.div_ceil(2));
    }
    n.checked_add(lf.saturating_sub(1))
        .map(|v| v / 2)
        .ok_or_else(|| "wdenoise: the requested size overflows".into())
}

/// Detail lengths from the finest level, then the approximation.
fn coeff_part_lengths(n: usize, level: usize, lf: usize, mode: Mode) -> Result<Vec<usize>> {
    let mut len = n;
    let mut parts = Vec::with_capacity(level + 1);
    for _ in 0..level {
        len = analysis_len(len, lf, mode)?;
        parts.push(len);
    }
    parts.push(len);
    Ok(parts)
}

fn matches_name(given: &str, full: &str) -> bool {
    given.eq_ignore_ascii_case(full)
}

fn wdenoise(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "wdenoise";
    need(args, 1, WHO)?;
    let x = real_matrix(&args[0], WHO)?;
    if x.is_nd() || x.is_empty() {
        return Err("wdenoise: X must be a nonempty real vector or matrix".into());
    }
    if x.data().iter().any(|v| !v.is_finite()) {
        return Err("wdenoise: X must contain finite values".into());
    }
    let row = x.rows() == 1;
    let (n, signals) = if row {
        (x.cols(), 1)
    } else {
        (x.rows(), x.cols())
    };
    if n < 2 {
        return Err("wdenoise: the signal must have at least 2 samples".into());
    }
    let mut rest = &args[1..];
    let mut level = None;
    if let Some(v) = rest.first()
        && text(v).is_none()
    {
        level = Some(integer(v, WHO, "the level")?);
        rest = &rest[1..];
    }
    if rest.len() % 2 == 1 {
        return Err("wdenoise: name-value arguments must come in pairs".into());
    }
    let mut wname = "sym4".to_string();
    let mut method = Method::Bayes;
    let mut method_name = "Bayes";
    let mut rule: Option<(ThRule, String)> = None;
    let mut level_dependent = false;
    for pair in rest.chunks(2) {
        let key = text(&pair[0]).ok_or("wdenoise: option names must be character vectors")?;
        let value = &pair[1];
        if matches_name(key, "Wavelet") {
            wname = text(value)
                .ok_or("wdenoise: 'Wavelet' must be a wavelet name")?
                .to_string();
        } else if matches_name(key, "DenoisingMethod") {
            (method, method_name) = denoising_method(e, value)?;
        } else if matches_name(key, "ThresholdRule") {
            let t = text(value).ok_or("wdenoise: 'ThresholdRule' must be a character vector")?;
            let r = match t.to_ascii_lowercase().as_str() {
                "median" => ThRule::Median,
                "mean" => ThRule::Mean,
                "soft" => ThRule::Soft,
                "hard" => ThRule::Hard,
                "james-stein" => ThRule::JamesStein,
                _ => return Err(format!("wdenoise: unknown ThresholdRule '{t}'")),
            };
            rule = Some((r, t.to_string()));
        } else if matches_name(key, "NoiseEstimate") {
            let t = text(value).ok_or("wdenoise: 'NoiseEstimate' must be a character vector")?;
            level_dependent = if t.eq_ignore_ascii_case("LevelDependent") {
                true
            } else if t.eq_ignore_ascii_case("LevelIndependent") {
                false
            } else {
                return Err(format!(
                    "wdenoise: NoiseEstimate must be 'LevelIndependent' or 'LevelDependent', not '{t}'"
                ));
            };
        } else {
            return Err(format!("wdenoise: unknown option '{key}'"));
        }
    }
    let allowed: &[ThRule] = match method {
        Method::Bayes => &[ThRule::Median, ThRule::Mean, ThRule::Soft, ThRule::Hard],
        Method::Fdr(_) => &[ThRule::Hard],
        Method::BlockJs => &[ThRule::JamesStein],
        _ => &[ThRule::Soft, ThRule::Hard],
    };
    let rule = match rule {
        None => allowed[0],
        Some((r, _)) if allowed.contains(&r) => r,
        Some((_, t)) => {
            return Err(format!(
                "wdenoise: ThresholdRule '{t}' is not valid for DenoisingMethod '{method_name}'"
            ));
        }
    };
    let bank = filters::bank(&wname)?;
    let top = (n as f64).log2().floor() as usize;
    let level = match level {
        Some(0) => return Err("wdenoise: the level must be a positive integer".into()),
        Some(l) if l > top => {
            return Err(format!(
                "wdenoise: the level must not exceed floor(log2(N)) = {top}"
            ));
        }
        Some(l) => l,
        None => top.min(transform::max_level(n, bank.lo_d.len())).max(1),
    };
    let mode = e.wavelet.mode;
    let lengths = coeff_part_lengths(n, level, bank.lo_d.len(), mode)?;
    let per_signal = lengths.iter().try_fold(0usize, |acc, &len| {
        acc.checked_add(len)
            .ok_or("wdenoise: the requested size overflows")
    })?;
    // xden, the coefficient cells nargout keeps, and one signal's scratch
    // (the coefficient vector, its clone, the copy, the reconstruction and
    // one detail) all exist together. Admit that before allocating any of it.
    let want_den = nargout > 1;
    let want_orig = nargout > 2;
    let signal_units = n
        .checked_mul(signals)
        .ok_or("wdenoise: the requested size overflows")?;
    let cell_units = per_signal
        .checked_mul(signals)
        .ok_or("wdenoise: the requested size overflows")?;
    let cells = usize::from(want_den) + usize::from(want_orig);
    let max_part = lengths.iter().copied().max().unwrap_or(0);
    let scratch = per_signal
        .checked_mul(2)
        .and_then(|v| v.checked_add(n))
        .and_then(|v| v.checked_add(n))
        .and_then(|v| v.checked_add(max_part.saturating_mul(2)))
        .ok_or("wdenoise: the requested size overflows")?;
    let units = signal_units
        .checked_add(
            cell_units
                .checked_mul(cells)
                .ok_or("wdenoise: the requested size overflows")?,
        )
        .and_then(|v| v.checked_add(scratch))
        .ok_or("wdenoise: the requested size overflows")?;
    e.preflight_elements(units)
        .map_err(|err| format!("wdenoise: {err}"))?;
    let mut xden = vec![0.; x.len()];
    // Per output cell: one column (or row) of coefficients per signal.
    // Built only for the outputs nargout asks for.
    let mut den_parts: Vec<Vec<f64>> = if want_den {
        lengths
            .iter()
            .map(|&len| Vec::with_capacity(len.saturating_mul(signals)))
            .collect()
    } else {
        Vec::new()
    };
    let mut orig_parts: Vec<Vec<f64>> = if want_orig {
        lengths
            .iter()
            .map(|&len| Vec::with_capacity(len.saturating_mul(signals)))
            .collect()
    } else {
        Vec::new()
    };
    let mut part_len = vec![0; level + 1];
    for s in 0..signals {
        let sig: Vec<f64> = if row {
            x.data().to_vec()
        } else {
            x.data()[s * n..(s + 1) * n].to_vec()
        };
        let (c, l) = transform::wavedec(&sig, level, &bank.lo_d, &bank.hi_d, mode, &mut || {
            e.check_cancel()
        })?;
        let sigma1 = denoise::noise_sigma(&transform::detail(&c, &l, 1));
        let mut cd = c.clone();
        for k in 1..=level {
            e.check_cancel()?;
            let d = transform::detail(&c, &l, k);
            let sigma = if level_dependent {
                denoise::noise_sigma(&d)
            } else {
                sigma1
            };
            let y = denoise::denoise_level(&d, sigma, method, rule, n)?;
            let start: usize = l[..=level - k].iter().sum();
            cd[start..start + y.len()].copy_from_slice(&y);
            if want_den {
                den_parts[k - 1].extend_from_slice(&y);
            }
            if want_orig {
                orig_parts[k - 1].extend_from_slice(&d);
            }
            part_len[k - 1] = d.len();
        }
        if want_den {
            den_parts[level].extend_from_slice(&c[..l[0]]);
        }
        if want_orig {
            orig_parts[level].extend_from_slice(&c[..l[0]]);
        }
        part_len[level] = l[0];
        let rec = transform::approx(&cd, &l, &bank.lo_r, &bank.hi_r, 0, mode, &mut || {
            e.check_cancel()
        })?;
        xden[s * n..(s + 1) * n].copy_from_slice(&rec);
    }
    let mut outs = vec![Value::Matrix(Matrix::from_column_vec(
        x.rows(),
        x.cols(),
        xden,
    )?)];
    if nargout > 1 {
        for parts in [den_parts, orig_parts].into_iter().take(nargout - 1) {
            let items = parts
                .into_iter()
                .zip(&part_len)
                .map(|(p, &len)| {
                    let (r, c) = if row { (1, len) } else { (len, signals) };
                    Ok(Value::Matrix(Matrix::from_column_vec(r, c, p)?))
                })
                .collect::<Result<Vec<_>>>()?;
            outs.push(cell_row(e, items)?);
        }
    }
    Ok(outs)
}

/// 'DenoisingMethod' value: a name, or {'FDR', Q} with 0 < Q <= 1/2.
fn denoising_method(e: &Engine, v: &Value) -> Result<(Method, &'static str)> {
    let (name, q) = match v {
        Value::Container(Array::Cell(c)) => {
            if c.len() != 2 {
                return Err("wdenoise: use {'FDR', Q} for a cell DenoisingMethod".into());
            }
            let name = match c.get_linear(0)?.as_ref() {
                Array::Character(t) => t.to_utf8(&e.cancel)?,
                _ => return Err("wdenoise: use {'FDR', Q} for a cell DenoisingMethod".into()),
            };
            let q = match c.get_linear(1)?.as_ref() {
                Array::Numeric(m) if m.len() == 1 => m.data()[0],
                _ => return Err("wdenoise: the FDR Q-value must be a scalar".into()),
            };
            if !name.eq_ignore_ascii_case("FDR") {
                return Err("wdenoise: only 'FDR' takes a parameter ({'FDR', Q})".into());
            }
            (name, Some(q))
        }
        other => (
            text(other)
                .ok_or("wdenoise: 'DenoisingMethod' must be a method name")?
                .to_string(),
            None,
        ),
    };
    Ok(match name.to_ascii_lowercase().as_str() {
        "bayes" => (Method::Bayes, "Bayes"),
        "sure" => (Method::Sure, "SURE"),
        "universalthreshold" => (Method::Universal, "UniversalThreshold"),
        "minimax" => (Method::Minimax, "Minimax"),
        "blockjs" => (Method::BlockJs, "BlockJS"),
        "fdr" => {
            let q = q.unwrap_or(0.05);
            if !(q > 0. && q <= 0.5) {
                return Err("wdenoise: the FDR Q-value must be in (0, 1/2]".into());
            }
            (Method::Fdr(q), "FDR")
        }
        _ => return Err(format!("wdenoise: unknown DenoisingMethod '{name}'")),
    })
}
