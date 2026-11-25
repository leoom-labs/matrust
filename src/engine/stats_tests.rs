//! Hypothesis tests and resampling. Formula references are recorded with each kernel.
use super::stats_dist::families::Family;
use super::{Engine, Execution, Value};
use crate::{Array, LogicalMatrix, Matrix, Result};
use std::sync::Arc;
mod anova;
mod chi;
mod ks;
mod rank;
mod sampling;
mod student;
const NAMES: &[&str] = &[
    "ttest",
    "ttest2",
    "anova1",
    "anova2",
    "kstest",
    "chi2gof",
    "ranksum",
    "signrank",
    "trimmean",
    "randsample",
    "datasample",
    "bootstrp",
    "mnrnd",
];
pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
impl Engine {
    pub(super) fn stt_call(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if args.is_empty() {
            return Err(format!("{name}: input arguments required"));
        }
        if matches!(&args[0], Value::Container(a) if super::classdef::object_class(a).as_deref()==Some("RandStream"))
        {
            return Err(format!("{name}: RandStream input is not supported yet"));
        }
        let max = match name {
            "ttest" | "ttest2" | "kstest" => 4,
            "anova1" | "anova2" | "chi2gof" | "ranksum" | "signrank" => 3,
            "bootstrp" | "datasample" => 2,
            _ => 1,
        };
        if count > max {
            return Err(format!("{name}: too many output arguments"));
        }
        let mut v = match name {
            "ttest" | "ttest2" => student::test(self, name, args)?,
            "trimmean" => vec![student::trim(self, args)?],
            "ranksum" | "signrank" => rank::test(self, name, args)?,
            "anova1" | "anova2" => anova::test(self, name, args, out)?,
            "kstest" => ks::test(self, args, count)?,
            "chi2gof" => chi::test(self, args, out)?,
            _ => sampling::call(self, name, args, out)?,
        };
        v.truncate(count.max(1));
        self.check_cancel()?;
        Ok(v)
    }
}
fn poll(e: &Engine, i: usize) -> Result<()> {
    if i.is_multiple_of(4096) {
        e.check_cancel()?;
    }
    Ok(())
}
fn mat(v: &Value, e: &Engine) -> Result<Matrix> {
    match v {
        Value::Matrix(m) => Ok(m.clone()),
        Value::Logical(m) | Value::Container(Array::Logical(m)) => m.to_numeric(&e.cancel),
        Value::Container(Array::Typed(t)) => t.to_matrix(),
        Value::Container(Array::Numeric(m)) => Ok(m.clone()),
        _ => Err("Expected real numeric or logical data".into()),
    }
}
fn vector(v: &Value, e: &Engine) -> Result<Vec<f64>> {
    let m = mat(v, e)?;
    if m.rows() > 1 && m.cols() > 1 {
        return Err("Input must be a vector".into());
    }
    let mut x = Vec::with_capacity(m.len());
    for (i, &v) in m.data().iter().enumerate() {
        poll(e, i)?;
        if !v.is_nan() {
            x.push(v)
        }
    }
    Ok(x)
}
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_ref().map(|s| s.to_string())
        }
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}
fn scalar(v: &Value, e: &Engine) -> Result<f64> {
    let m = mat(v, e)?;
    if m.len() != 1 {
        return Err("Expected a numeric scalar".into());
    }
    Ok(m.data()[0])
}
fn integer(v: &Value, e: &Engine) -> Result<usize> {
    let n = scalar(v, e)?;
    if !n.is_finite()
        || n < 0.
        || n.fract() != 0.
        || n > 9_007_199_254_740_992f64.min(usize::MAX as f64)
    {
        return Err("Expected a nonnegative scalar integer".into());
    }
    Ok(n as usize)
}
fn empty(v: &Value) -> bool {
    v.dims().iter().product::<usize>() == 0
}
fn val(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}
fn matrix(r: usize, c: usize, x: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(r, c, x)?))
}
fn row(x: Vec<f64>) -> Result<Value> {
    matrix(1, x.len(), x)
}
fn logical(x: bool) -> Value {
    Value::Logical(LogicalMatrix::scalar(x))
}
fn structure(e: &Engine, fields: Vec<(&str, Value)>) -> Result<Value> {
    let mut s = crate::StructArray::scalar_empty(&e.cancel)?;
    for (n, v) in fields {
        s = s.with_field(n, v.array(&e.cancel)?, &e.cancel)?;
    }
    Ok(Value::Container(Array::Struct(s)))
}
fn cell(e: &Engine, r: usize, c: usize, v: Vec<Value>) -> Result<Value> {
    let a = v
        .into_iter()
        .map(|v| v.array(&e.cancel).map(Arc::new))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(
        crate::CellArray::from_shared(r, c, a, &e.cancel)?,
    )))
}
fn blank() -> Value {
    Value::Matrix(Matrix::from_column_vec(0, 0, vec![]).expect("empty"))
}
fn mean_sd(e: &Engine, x: &[f64]) -> Result<(f64, f64)> {
    if x.is_empty() {
        return Ok((f64::NAN, f64::NAN));
    }
    let base = x[0];
    let mut sum = 0.;
    for (i, &v) in x.iter().enumerate() {
        poll(e, i)?;
        sum += v - base;
    }
    let mean = base + sum / x.len() as f64;
    let mut variance = 0.;
    for (i, &v) in x.iter().enumerate() {
        poll(e, i)?;
        variance += (v - mean).powi(2);
    }
    Ok((
        mean,
        if x.len() < 2 {
            f64::NAN
        } else {
            (variance / (x.len() - 1) as f64).sqrt()
        },
    ))
}
#[derive(Default)]
struct Opts {
    pairs: Vec<(String, Value)>,
}
impl Opts {
    fn parse(args: &[Value], start: usize, names: &[&str]) -> Result<(usize, Self)> {
        let mut end = args.len();
        let mut pairs = Vec::new();
        while end >= start + 2 {
            let Some(s) = text(&args[end - 2]) else { break };
            let key = s.to_ascii_lowercase();
            let hits: Vec<_> = names
                .iter()
                .filter(|n| n.to_ascii_lowercase().starts_with(&key))
                .collect();
            if hits.len() != 1 {
                break;
            }
            let n = hits[0].to_ascii_lowercase();
            if pairs.iter().any(|(k, _)| k == &n) {
                return Err(format!("Duplicate option {n}"));
            }
            pairs.push((n, args[end - 1].clone()));
            end -= 2;
        }
        Ok((end, Self { pairs }))
    }
    fn get(&self, k: &str) -> Option<&Value> {
        self.pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v)
    }
    fn num(&self, k: &str, default: f64, e: &Engine) -> Result<f64> {
        self.get(k).map_or(Ok(default), |v| scalar(v, e))
    }
    fn word(&self, k: &str, default: &str) -> Result<String> {
        self.get(k).map_or(Ok(default.into()), |v| {
            text(v)
                .map(|s| s.to_ascii_lowercase())
                .ok_or_else(|| format!("{k}: expected text"))
        })
    }
}
fn alpha(a: f64) -> Result<f64> {
    if a > 0. && a < 1. {
        Ok(a)
    } else {
        Err("Alpha must be between zero and one".into())
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Tail {
    Both,
    Left,
    Right,
}
fn tail(s: &str) -> Result<Tail> {
    match s.to_ascii_lowercase().as_str() {
        "both" => Ok(Tail::Both),
        "left" => Ok(Tail::Left),
        "right" => Ok(Tail::Right),
        _ => Err("Tail must be both, left or right".into()),
    }
}
fn probability(f: Family, t: f64, q: &[f64], tail: Tail) -> f64 {
    if t.is_nan() {
        return f64::NAN;
    }
    let p = match tail {
        Tail::Both => 2. * f.cdf(t.abs(), q, true),
        Tail::Left => f.cdf(t, q, false),
        Tail::Right => f.cdf(t, q, true),
    };
    if p.is_nan() { p } else { p.clamp(0., 1.) }
}

fn decision(p: f64, a: f64) -> f64 {
    if p.is_nan() {
        f64::NAN
    } else {
        f64::from(p < a)
    }
}
fn dimension(m: &Matrix, v: Option<&Value>, e: &Engine) -> Result<usize> {
    let d = if let Some(v) = v {
        if empty(v) {
            if m.rows() == 1 { 2 } else { 1 }
        } else {
            integer(v, e)?
        }
    } else if m.rows() == 1 {
        2
    } else {
        1
    };
    if d != 1 && d != 2 {
        return Err("Dim: only dimensions 1 and 2 are supported yet".into());
    }
    Ok(d)
}
fn lanes(e: &Engine, m: &Matrix, d: usize) -> Result<Vec<Vec<f64>>> {
    let (n, k) = if d == 1 {
        (m.rows(), m.cols())
    } else {
        (m.cols(), m.rows())
    };
    let mut out = Vec::with_capacity(k);
    for j in 0..k {
        poll(e, j)?;
        let mut x = Vec::with_capacity(n);
        for i in 0..n {
            poll(e, i)?;
            x.push(if d == 1 {
                m.data()[i + j * m.rows()]
            } else {
                m.data()[j + i * m.rows()]
            })
        }
        out.push(x)
    }
    Ok(out)
}
fn reduced(d: usize, x: Vec<f64>) -> Result<Value> {
    if d == 1 {
        row(x)
    } else {
        matrix(x.len(), 1, x)
    }
}
fn interval(d: usize, lo: Vec<f64>, hi: Vec<f64>) -> Result<Value> {
    let n = lo.len();
    Matrix::checked_len(2, n)?;
    if d == 1 {
        matrix(
            2,
            n,
            lo.into_iter().zip(hi).flat_map(|(a, b)| [a, b]).collect(),
        )
    } else {
        matrix(n, 2, lo.into_iter().chain(hi).collect())
    }
}

/// Bounded runs keep cancellation observable during long ordering operations.
fn sort<T: Clone>(
    e: &Engine,
    data: &mut [T],
    cmp: impl Fn(&T, &T) -> std::cmp::Ordering,
) -> Result<()> {
    const RUN: usize = 4096;
    for chunk in data.chunks_mut(RUN) {
        e.check_cancel()?;
        chunk.sort_by(&cmp);
    }
    if data.len() <= RUN {
        return e.check_cancel();
    }
    let mut tmp = Vec::with_capacity(data.len());
    for chunk in data.chunks(RUN) {
        e.check_cancel()?;
        tmp.extend_from_slice(chunk);
    }
    let mut width = RUN;
    while width < data.len() {
        for start in (0..data.len()).step_by(2 * width) {
            let mid = (start + width).min(data.len());
            let end = (start + 2 * width).min(data.len());
            let (mut a, mut b) = (start, mid);
            for (i, slot) in tmp.iter_mut().enumerate().take(end).skip(start) {
                poll(e, i)?;
                if a < mid && (b == end || cmp(&data[a], &data[b]) != std::cmp::Ordering::Greater) {
                    *slot = data[a].clone();
                    a += 1
                } else {
                    *slot = data[b].clone();
                    b += 1
                }
            }
        }
        for (a, b) in data.chunks_mut(RUN).zip(tmp.chunks(RUN)) {
            e.check_cancel()?;
            a.clone_from_slice(b);
        }
        width *= 2;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stats_tests_cancellation_reaches_kernels() {
        let e = Engine::default();
        e.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(ks::sf(&e, 10, 0.274, true).is_err());
        assert!(
            rank::test(
                &e,
                "ranksum",
                &[row(vec![1., 2.]).unwrap(), row(vec![3., 4.]).unwrap()]
            )
            .is_err()
        );
    }
}
