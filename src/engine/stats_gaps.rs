//! Core statistics gaps: prctile, quantile, zscore, skewness, kurtosis, mad,
//! normalize, rescale, smoothdata and bounds on real 2-D data, and rms.
//!
//! Definitions (published mathematics, behaviour from the public MathWorks
//! reference pages; no MathWorks code):
//! - Percentiles and quantiles use the midpoint rule: the k-th of n sorted
//!   values is the (k - 0.5)/n quantile, with linear interpolation between and
//!   the extremes outside (Hazen 1914; Hyndman & Fan, "Sample quantiles in
//!   statistical packages", The American Statistician 50(4), 1996, definition 5).
//! - Skewness and kurtosis are the moment ratios m3/m2^1.5 and m4/m2^2 and
//!   their standard bias corrections (Joanes & Gill, "Comparing measures of
//!   sample skewness and kurtosis", The Statistician 47(1), 1998, G1 and G2+3).
//! - lowess/loess are locally weighted polynomial fits with tricube weights
//!   and bisquare robustness weights (Cleveland, "Robust locally weighted
//!   regression and smoothing scatterplots", JASA 74(368), 1979). sgolay is
//!   the unweighted local polynomial fit (Savitzky & Golay, Anal. Chem. 36(8), 1964).
//!
//! Every function here is a free function; the one `impl Engine` method uses
//! the `stats_gaps_` prefix so parallel tracks cannot collide with it.
use super::{Engine, Value};
use crate::{Array, MATLAB_NAN, Matrix, NumericClass, Result, TypedArray};
use nalgebra::{DMatrix, DVector};

const NAMES: &[&str] = &[
    "prctile",
    "quantile",
    "zscore",
    "skewness",
    "kurtosis",
    "mad",
    "normalize",
    "rescale",
    "smoothdata",
    "bounds",
    "rms",
];

/// Functions MATLAB ships as program files: `exist` gives 2 for them.
pub(super) const PROGRAM_FILES: &[&str] = &["rms"];

/// Elements per cancellation poll.
const POLL: usize = 4096;
/// Output elements a statement may create (the engine's 256 MiB budget).
const BUDGET: usize = 32 * 1024 * 1024;
/// Multiply-adds one smoothdata call may spend on local fits.
const FIT_WORK: u128 = 4_000_000_000;
/// Robustness passes of rlowess and rloess after the first fit.
const ROBUST_PASSES: usize = 5;
/// Default smoothing factor of smoothdata.
const DEFAULT_FACTOR: f64 = 0.25;

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn max_outputs(name: &str) -> usize {
    match name {
        "zscore" => 3,
        "bounds" | "smoothdata" => 2,
        _ => 1,
    }
}

impl Engine {
    /// Dispatch for every function of this module; `count` is the number of
    /// requested outputs (1 for an expression).
    pub(super) fn stats_gaps_call(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let max = max_outputs(name);
        if count > max {
            return Err(format!("{name}: too many output arguments (at most {max})"));
        }
        if args.is_empty() {
            return Err(format!("{name} requires at least one input argument"));
        }
        // Class rule of the data argument (one place for every output count).
        let class = typed_class(&args[0]);
        if let Some(c) = class
            && c.is_integer()
            && FLOAT_ONLY.contains(&name)
        {
            return Err(format!(
                "{name} requires single or double input, not {}",
                c.name()
            ));
        }
        // Integer bounds select natively (exact for every integer class).
        let native = name == "bounds" && class.is_some_and(NumericClass::is_integer);
        // Option strings in any text form; the data argument keeps its value.
        let mut normalized = vec![if native {
            args[0].clone()
        } else {
            as_double(args[0].clone())?
        }];
        for v in self.text_arguments(args[1..].to_vec())? {
            normalized.push(as_double(v)?);
        }
        let args = normalized.as_slice();
        let mut values = match name {
            "prctile" => vec![percentiles(self, args, true)?],
            "quantile" => vec![percentiles(self, args, false)?],
            "zscore" => zscore(self, args)?,
            "skewness" | "kurtosis" => vec![moments(self, name, args)?],
            "mad" => vec![mad(self, args)?],
            "normalize" => vec![normalize(self, args)?],
            "rescale" => vec![rescale(self, args)?],
            "smoothdata" => smoothdata(self, args)?,
            "bounds" => bounds(self, args, count)?,
            "rms" => vec![rms(self, args)?],
            _ => unreachable!("checked by handles"),
        };
        self.check_cancel()?;
        values.truncate(count.max(1));
        // Single data gives single results (computed in double, then rounded);
        // the window output of smoothdata stays double.
        if class == Some(NumericClass::Single) {
            for (i, v) in values.iter_mut().enumerate() {
                if let Value::Matrix(m) = v
                    && !(name == "smoothdata" && i == 1)
                {
                    *v = Value::Container(Array::Typed(TypedArray::from_matrix(
                        NumericClass::Single,
                        m,
                    )?));
                }
            }
        }
        Ok(values)
    }
}

/// Functions whose published input types are single and double only.
const FLOAT_ONLY: &[&str] = &[
    "prctile",
    "quantile",
    "zscore",
    "skewness",
    "kurtosis",
    "mad",
    "normalize",
    "rms",
];

fn typed_class(value: &Value) -> Option<NumericClass> {
    match value {
        Value::Container(Array::Typed(t)) => Some(t.class()),
        _ => None,
    }
}

/// Integer and single values as doubles; every other value unchanged.
fn as_double(value: Value) -> Result<Value> {
    match value {
        Value::Container(Array::Typed(t)) => Ok(Value::Matrix(t.to_matrix()?)),
        v => Ok(v),
    }
}

// ---------------------------------------------------------------- helpers

pub(super) fn poll(e: &Engine, i: usize) -> Result<()> {
    if i.is_multiple_of(POLL) {
        e.check_cancel()?;
    }
    Ok(())
}

pub(super) fn budget(e: &Engine, elements: usize) -> Result<()> {
    if e.evaluated_elements
        .checked_add(elements)
        .is_none_or(|n| n > BUDGET)
    {
        return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
    }
    Ok(())
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Text(t) => Some(t.to_ascii_lowercase()),
        _ => None,
    }
}

fn numeric(value: &Value) -> Option<&Matrix> {
    match value {
        Value::Matrix(m) => Some(m),
        _ => None,
    }
}

/// Real double data of the data argument (logical converts to double).
fn real_data(e: &Engine, value: &Value, name: &str) -> Result<Matrix> {
    let m = match value {
        Value::Matrix(m) => m.clone(),
        Value::Logical(l) => l.to_numeric(&e.cancel)?,
        Value::Complex(_) => return Err(format!("{name} of complex input is not supported yet")),
        _ => return Err(format!("{name} requires real numeric or logical input")),
    };
    if m.is_nd() {
        return Err(format!(
            "{name} of arrays with more than two dimensions is not supported yet"
        ));
    }
    Ok(m)
}

/// First dimension whose size is not 1 (1 for a scalar or []).
fn default_dim(m: &Matrix) -> usize {
    if m.rows() != 1 { 1 } else { 2 }
}

fn dimension(value: &Value, name: &str) -> Result<usize> {
    let d = match numeric(value) {
        Some(m) if m.len() == 1 => m.data()[0],
        _ => {
            return Err(format!(
                "{name} dimension must be a positive integer scalar; 'all' and vector dimensions are not supported yet"
            ));
        }
    };
    if !(d >= 1. && d.fract() == 0. && d <= crate::matrix::MAX_ELEMENTS as f64) {
        return Err(format!(
            "{name} dimension must be a positive integer scalar"
        ));
    }
    Ok(d as usize)
}

/// Lanes (columns, rows or single elements) of a column-major matrix.
#[derive(Clone, Copy)]
pub(super) struct Lanes {
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) dim: usize,
}

impl Lanes {
    pub(super) fn new(m: &Matrix, dim: usize) -> Self {
        Self {
            rows: m.rows(),
            cols: m.cols(),
            dim,
        }
    }
    pub(super) fn count(&self) -> usize {
        match self.dim {
            1 => self.cols,
            2 => self.rows,
            _ => self.rows * self.cols,
        }
    }
    pub(super) fn len(&self) -> usize {
        match self.dim {
            1 => self.rows,
            2 => self.cols,
            _ => 1,
        }
    }
    /// Storage index of element `i` of lane `l` for a lane length `len`.
    pub(super) fn index(&self, l: usize, i: usize, len: usize) -> usize {
        match self.dim {
            1 => l * len + i,
            2 => l + i * self.rows,
            _ => l,
        }
    }
    pub(super) fn lane(&self, data: &[f64], l: usize, out: &mut Vec<f64>) {
        out.clear();
        let len = self.len();
        out.extend((0..len).map(|i| data[self.index(l, i, len)]));
    }
    /// Shape of a result with `len` values per lane.
    pub(super) fn shape(&self, len: usize) -> (usize, usize) {
        match self.dim {
            1 => (len, self.cols),
            2 => (self.rows, len),
            _ => (self.rows, self.cols),
        }
    }
}

/// Apply `f` to every lane; `f` writes `len` values per lane.
fn map_lanes(
    e: &Engine,
    x: &Matrix,
    dim: usize,
    len: usize,
    mut f: impl FnMut(&[f64], &mut [f64]) -> Result<()>,
) -> Result<Matrix> {
    let lanes = Lanes::new(x, dim);
    if dim > 2 && len != 1 && len != lanes.len() {
        return Err("Results along dimension 3 or higher with several values per lane are not supported yet".into());
    }
    let (rows, cols) = lanes.shape(len);
    let total = Matrix::checked_len(rows, cols)?;
    budget(e, total)?;
    let mut out = vec![0.; total];
    let mut lane = Vec::with_capacity(lanes.len());
    let mut result = vec![0.; len];
    let out_lanes = Lanes { rows, cols, dim };
    for l in 0..lanes.count() {
        poll(e, l)?;
        lanes.lane(x.data(), l, &mut lane);
        f(&lane, &mut result)?;
        for (i, &v) in result.iter().enumerate() {
            out[out_lanes.index(l, i, len)] = v;
        }
    }
    Matrix::from_column_vec(rows, cols, out)
}

/// The canonical empty [] reduces to one NaN, as mean([]) does.
fn reduction_input(x: Matrix, dim_given: bool) -> Result<Matrix> {
    if !dim_given && x.rows() == 0 && x.cols() == 0 {
        Matrix::filled(0, 1, 0.)
    } else {
        Ok(x)
    }
}

fn present(lane: &[f64]) -> Vec<f64> {
    lane.iter().copied().filter(|v| !v.is_nan()).collect()
}

fn sorted_present(lane: &[f64]) -> Vec<f64> {
    let mut v = present(lane);
    v.sort_unstable_by(f64::total_cmp);
    v
}

pub(super) fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return MATLAB_NAN;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

/// Standard deviation; `population` divides by n, otherwise by n - 1 (n for n = 1).
pub(super) fn std_dev(v: &[f64], population: bool) -> f64 {
    let n = v.len();
    if n == 0 {
        return MATLAB_NAN;
    }
    let mu = mean(v);
    let ss: f64 = v.iter().map(|x| (x - mu) * (x - mu)).sum();
    let d = if population || n == 1 { n } else { n - 1 };
    (ss / d as f64).sqrt()
}

pub(super) fn median_sorted(s: &[f64]) -> f64 {
    let n = s.len();
    if n == 0 {
        return MATLAB_NAN;
    }
    if n % 2 == 1 {
        s[n / 2]
    } else {
        let (a, b) = (s[n / 2 - 1], s[n / 2]);
        let m = (a + b) / 2.;
        if m.is_finite() || !a.is_finite() || !b.is_finite() {
            m
        } else {
            a / 2. + b / 2.
        }
    }
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_unstable_by(f64::total_cmp);
    median_sorted(&s)
}

/// Median absolute deviation from the median (no consistency constant).
fn median_abs_dev(v: &[f64]) -> f64 {
    let m = median(v);
    let d: Vec<f64> = v.iter().map(|x| (x - m).abs()).collect();
    median(&d)
}

/// Midpoint-rule quantile of sorted data at the 1-based position `r`
/// (r = n q + 1/2): extremes outside [1, n], linear interpolation inside.
pub(super) fn midpoint_at(sorted: &[f64], r: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return MATLAB_NAN;
    }
    if r <= 1. {
        return sorted[0];
    }
    if r >= n as f64 {
        return sorted[n - 1];
    }
    let k = r.floor();
    let f = r - k;
    let lo = sorted[k as usize - 1];
    let hi = sorted[k as usize];
    if f == 0. || lo == hi {
        lo
    } else {
        lo + f * (hi - lo)
    }
}

/// Interquartile range by the midpoint rule.
fn iqr_sorted(s: &[f64]) -> f64 {
    let n = s.len() as f64;
    midpoint_at(s, n * 0.75 + 0.5) - midpoint_at(s, n * 0.25 + 0.5)
}

// ------------------------------------------------------ prctile, quantile

fn percentiles(e: &Engine, args: &[Value], percent: bool) -> Result<Value> {
    let name = if percent { "prctile" } else { "quantile" };
    if !(2..=3).contains(&args.len()) {
        return Err(format!("Use {name}(X, p) or {name}(X, p, dim)"));
    }
    let x = real_data(e, &args[0], name)?;
    let p_value = real_data(e, &args[1], name)?;
    if (p_value.rows() != 1 && p_value.cols() != 1) || p_value.is_empty() {
        return Err(format!("{name}: p must be a nonempty vector"));
    }
    let mut p = p_value.data().to_vec();
    let mut p_rows = p_value.rows();
    let limit = if percent { 100. } else { 1. };
    // quantile(X, N): N evenly spaced cumulative probabilities.
    if !percent && p.len() == 1 && p[0] > 1. && p[0].fract() == 0. && p[0] <= 1e6 {
        let n = p[0] as usize;
        p = (1..=n).map(|k| k as f64 / (n + 1) as f64).collect();
        p_rows = 1;
    } else if p.iter().any(|v| !(0. ..=limit).contains(v)) {
        return Err(if percent {
            "prctile: percentages must be real values between 0 and 100".into()
        } else {
            "quantile: probabilities must be real values between 0 and 1, or an integer N > 1"
                .into()
        });
    }
    let dim_given = args.len() == 3;
    let dim = if dim_given {
        dimension(&args[2], name)?
    } else {
        default_dim(&x)
    };
    let vector = !dim_given && (x.rows() == 1 || x.cols() == 1 || (x.rows() == 0 && x.cols() == 0));
    let x = reduction_input(x, dim_given)?;
    let np = p.len();
    let position = |n: usize, q: f64| {
        if percent {
            n as f64 * q / 100. + 0.5
        } else {
            n as f64 * q + 0.5
        }
    };
    let compute = |lane: &[f64], out: &mut [f64]| -> Result<()> {
        let s = sorted_present(lane);
        for (slot, &q) in out.iter_mut().zip(&p) {
            *slot = midpoint_at(&s, position(s.len(), q));
        }
        Ok(())
    };
    if vector {
        let mut out = vec![0.; np];
        compute(x.data(), &mut out)?;
        let (rows, cols) = if p_rows == 1 { (1, np) } else { (np, 1) };
        return Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, out)?));
    }
    Ok(Value::Matrix(map_lanes(e, &x, dim, np, compute)?))
}

// ------------------------------------------------------------------ zscore

/// Normalization flag: [] or 0 gives n - 1, 1 gives n.
fn norm_flag(value: &Value, name: &str, default: bool) -> Result<bool> {
    match numeric(value) {
        Some(m) if m.is_empty() => Ok(default),
        Some(m) if m.len() == 1 && m.data()[0] == 0. => Ok(false),
        Some(m) if m.len() == 1 && m.data()[0] == 1. => Ok(true),
        _ => Err(format!("{name} flag must be 0, 1 or []")),
    }
}

fn zscore(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() > 3 {
        return Err("Use zscore(X), zscore(X, flag) or zscore(X, flag, dim)".into());
    }
    let x = real_data(e, &args[0], "zscore")?;
    let population = match args.get(1) {
        Some(v) => norm_flag(v, "zscore", false)?,
        None => false,
    };
    let dim = match args.get(2) {
        Some(d) => dimension(d, "zscore")?,
        None => default_dim(&x),
    };
    let stats_x = reduction_input(x.clone(), args.len() == 3)?;
    let mu = map_lanes(e, &stats_x, dim, 1, |lane, out| {
        out[0] = mean(lane);
        Ok(())
    })?;
    let sigma = map_lanes(e, &stats_x, dim, 1, |lane, out| {
        out[0] = std_dev(lane, population);
        Ok(())
    })?;
    let lanes = Lanes::new(&x, dim);
    let mut l = 0;
    let z = map_lanes(e, &x, dim, lanes.len(), |lane, out| {
        let (m, s) = (mu.data()[l], sigma.data()[l]);
        // A zero standard deviation divides by 1, so constant data gives 0.
        let s = if s == 0. { 1. } else { s };
        for (o, v) in out.iter_mut().zip(lane) {
            *o = (v - m) / s;
        }
        l += 1;
        Ok(())
    })?;
    Ok(vec![
        Value::Matrix(z),
        Value::Matrix(mu),
        Value::Matrix(sigma),
    ])
}

// ------------------------------------------------- skewness, kurtosis, mad

/// Parse (X[, flag[, dim]]) with a flag that is 0, 1 or [].
fn flag_dim(
    e: &Engine,
    args: &[Value],
    name: &str,
    default: f64,
) -> Result<(Matrix, f64, usize, bool)> {
    if args.len() > 3 {
        return Err(format!(
            "Use {name}(X), {name}(X, flag) or {name}(X, flag, dim)"
        ));
    }
    let x = real_data(e, &args[0], name)?;
    let flag = match args.get(1).map(numeric) {
        None => default,
        Some(Some(m)) if m.is_empty() => default,
        Some(Some(m)) if m.len() == 1 && (m.data()[0] == 0. || m.data()[0] == 1.) => m.data()[0],
        _ => return Err(format!("{name} flag must be 0, 1 or []")),
    };
    let dim = match args.get(2) {
        Some(d) => dimension(d, name)?,
        None => default_dim(&x),
    };
    Ok((x, flag, dim, args.len() == 3))
}

/// Central moments m2, m3, m4 (divided by n) of NaN-free data.
fn central_moments(v: &[f64]) -> (f64, f64, f64) {
    let n = v.len() as f64;
    let mu = mean(v);
    let (mut m2, mut m3, mut m4) = (0., 0., 0.);
    for x in v {
        let d = x - mu;
        let d2 = d * d;
        m2 += d2;
        m3 += d2 * d;
        m4 += d2 * d2;
    }
    (m2 / n, m3 / n, m4 / n)
}

fn moments(e: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    let (x, flag, dim, dim_given) = flag_dim(e, args, name, 1.)?;
    let x = reduction_input(x, dim_given)?;
    let biased = flag == 1.;
    let skew = name == "skewness";
    Ok(Value::Matrix(map_lanes(e, &x, dim, 1, |lane, out| {
        let v = present(lane);
        let n = v.len() as f64;
        out[0] = if v.is_empty() {
            MATLAB_NAN
        } else {
            let (m2, m3, m4) = central_moments(&v);
            if skew {
                let s1 = m3 / m2.powf(1.5);
                if biased {
                    s1
                } else if v.len() < 3 {
                    MATLAB_NAN
                } else {
                    s1 * (n * (n - 1.)).sqrt() / (n - 2.)
                }
            } else {
                let k1 = m4 / (m2 * m2);
                if biased {
                    k1
                } else if v.len() < 4 {
                    MATLAB_NAN
                } else {
                    (n - 1.) / ((n - 2.) * (n - 3.)) * ((n + 1.) * k1 - 3. * (n - 1.)) + 3.
                }
            }
        };
        Ok(())
    })?))
}

fn mad(e: &Engine, args: &[Value]) -> Result<Value> {
    let (x, flag, dim, dim_given) = flag_dim(e, args, "mad", 0.)?;
    let x = reduction_input(x, dim_given)?;
    Ok(Value::Matrix(map_lanes(e, &x, dim, 1, |lane, out| {
        let v = present(lane);
        out[0] = if flag == 0. {
            let mu = mean(&v);
            let d: Vec<f64> = v.iter().map(|x| (x - mu).abs()).collect();
            mean(&d)
        } else {
            median_abs_dev(&v)
        };
        Ok(())
    })?))
}

// ---------------------------------------------------------------- rms

/// rms(X[, dim | vecdim | 'all'][, nanflag]): sqrt(mean(abs(X).^2, ...)).
/// The squares go through the `mean` reduction, which owns the dimension,
/// vecdim, 'all' and NaN rules for N-D data; only those options pass.
fn rms(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() > 3 {
        return Err("Use rms(X), rms(X, dim), rms(X, vecdim), rms(X, 'all') and an optional 'omitnan' or 'includenan'".into());
    }
    let x = &args[0];
    let dims = x.dims().into_owned();
    budget(e, dims.iter().product())?;
    let squares: Vec<f64> = match x {
        Value::Matrix(m) => m.data().iter().map(|v| v * v).collect(),
        Value::Logical(l) => l.data().iter().map(|&b| f64::from(b)).collect(),
        Value::Complex(z) => z.data().iter().map(|z| z.norm_sqr()).collect(),
        Value::Container(Array::Character(c)) => {
            c.units().iter().map(|&u| f64::from(u).powi(2)).collect()
        }
        Value::Text(t) => t.encode_utf16().map(|u| f64::from(u).powi(2)).collect(),
        _ => return Err("rms requires numeric, logical or char input".into()),
    };
    for (i, option) in args[1..].iter().enumerate() {
        match text(option).as_deref() {
            None => {}
            Some("all") if i == 0 => {}
            Some("omitnan" | "includenan" | "omitmissing" | "includemissing") => {}
            Some(other) => {
                return Err(format!(
                    "rms: unknown option '{other}'; use dim, vecdim, 'all', 'omitnan' or 'includenan'"
                ));
            }
        }
    }
    let dimension_given = args[1..].iter().any(|v| text(v).is_none_or(|t| t == "all"));
    if dims == [0, 0] && !dimension_given {
        // rms([]) is NaN, like mean([]).
        return Ok(Value::Matrix(Matrix::scalar(MATLAB_NAN)));
    }
    let cols = dims[1..].iter().product();
    let squares = Matrix::from_column_vec(dims[0], cols, squares)?.with_dims(&dims)?;
    let mut call = vec![Value::Matrix(squares)];
    call.extend(args[1..].iter().cloned());
    match super::nd_reduce::reduce(e, "mean", &call, false)?.0 {
        Value::Matrix(m) => Ok(Value::Matrix(m.map(f64::sqrt).with_dims(&m.dims())?)),
        other => Ok(other),
    }
}

// ---------------------------------------------------------------- normalize

/// Location or scale given by the caller: one value, or one per lane.
#[derive(Clone)]
enum Given {
    One(f64),
    PerLane(Vec<f64>),
}

impl Given {
    fn at(&self, l: usize) -> f64 {
        match self {
            Self::One(v) => *v,
            Self::PerLane(v) => v[l],
        }
    }
}

#[derive(Clone)]
enum Center {
    None,
    Mean,
    Median,
    Value(Given),
}

#[derive(Clone)]
enum Scale {
    None,
    Std,
    Mad,
    First,
    Iqr,
    Norm(f64),
    Value(Given),
}

fn given(value: &Value, what: &str) -> Result<Given> {
    match numeric(value) {
        Some(m) if m.len() == 1 => Ok(Given::One(m.data()[0])),
        Some(m) if (m.rows() == 1 || m.cols() == 1) && !m.is_empty() => {
            Ok(Given::PerLane(m.data().to_vec()))
        }
        _ => Err(format!(
            "normalize {what} must be a numeric scalar or one value per vector being normalized"
        )),
    }
}

fn normalize(e: &Engine, args: &[Value]) -> Result<Value> {
    let usage = "Use normalize(A[, dim][, method[, type]]) with method 'zscore', 'norm', 'scale', 'range', 'center' or 'medianiqr', or normalize(A[, dim], 'center', c, 'scale', s)";
    let x = real_data(e, &args[0], "normalize")?;
    let mut at = 1;
    let mut dim = default_dim(&x);
    if let Some(v @ Value::Matrix(_)) = args.get(1) {
        dim = dimension(v, "normalize")?;
        at = 2;
    }
    let mut center = Center::Mean;
    let mut scale = Scale::Std;
    let mut range: Option<(f64, f64)> = None;
    let mut methods = Vec::new();
    while at < args.len() {
        let Some(method) = text(&args[at]) else {
            return Err(usage.into());
        };
        let next = args.get(at + 1);
        let next_text = next.and_then(text);
        let next_num = next.and_then(numeric);
        at += 1;
        let mut took = true;
        match method.as_str() {
            "zscore" => {
                center = Center::Mean;
                scale = Scale::Std;
                match next_text.as_deref() {
                    Some("std") => {}
                    Some("robust") => {
                        center = Center::Median;
                        scale = Scale::Mad;
                    }
                    _ => took = false,
                }
            }
            "norm" => {
                center = Center::None;
                scale = Scale::Norm(2.);
                if let Some(m) = next_num {
                    let p = if m.len() == 1 { m.data()[0] } else { f64::NAN };
                    if p.is_nan() || p <= 0. {
                        return Err("normalize 'norm' type must be a positive number or Inf".into());
                    }
                    scale = Scale::Norm(p);
                } else {
                    took = false;
                }
            }
            "scale" => {
                if !methods.iter().any(|m| m == "center") {
                    center = Center::None;
                }
                scale = Scale::Std;
                match (next_text.as_deref(), next_num) {
                    (Some("std"), _) => {}
                    (Some("mad"), _) => scale = Scale::Mad,
                    (Some("first"), _) => scale = Scale::First,
                    (Some("iqr"), _) => scale = Scale::Iqr,
                    (None, Some(_)) => scale = Scale::Value(given(next.unwrap(), "scale")?),
                    _ => took = false,
                }
            }
            "range" => {
                center = Center::None;
                scale = Scale::None;
                range = Some((0., 1.));
                if let Some(m) = next_num {
                    if m.len() != 2 {
                        return Err(
                            "normalize 'range' type must be a two-element vector [a b]".into()
                        );
                    }
                    range = Some((m.data()[0], m.data()[1]));
                } else {
                    took = false;
                }
            }
            "center" => {
                if !methods.iter().any(|m| m == "scale") {
                    scale = Scale::None;
                }
                center = Center::Mean;
                match (next_text.as_deref(), next_num) {
                    (Some("mean"), _) => {}
                    (Some("median"), _) => center = Center::Median,
                    (None, Some(_)) => center = Center::Value(given(next.unwrap(), "center")?),
                    _ => took = false,
                }
            }
            "medianiqr" => {
                center = Center::Median;
                scale = Scale::Iqr;
                took = false;
            }
            "datavariables" | "replacevalues" => {
                return Err(format!(
                    "normalize option '{method}' applies to tables, which are not supported yet"
                ));
            }
            _ => return Err(usage.into()),
        }
        if took {
            at += 1;
        }
        methods.push(method);
    }
    let combined = methods.len() == 2
        && methods.iter().any(|m| m == "center")
        && methods.iter().any(|m| m == "scale");
    if methods.len() > 1 && !combined {
        return Err(usage.into());
    }
    let lanes = Lanes::new(&x, dim);
    let check = |g: &Given, what: &str| -> Result<()> {
        if let Given::PerLane(v) = g
            && v.len() != lanes.count()
        {
            return Err(format!(
                "normalize {what} must be a scalar or have one value per vector being normalized ({})",
                lanes.count()
            ));
        }
        Ok(())
    };
    if let Center::Value(g) = &center {
        check(g, "center")?;
    }
    if let Scale::Value(g) = &scale {
        check(g, "scale")?;
    }
    let mut l = 0;
    Ok(Value::Matrix(map_lanes(
        e,
        &x,
        dim,
        lanes.len(),
        |lane, out| {
            let v = present(lane);
            if let Some((a, b)) = range {
                let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                for (o, &y) in out.iter_mut().zip(lane) {
                    *o = if y.is_nan() || v.is_empty() {
                        MATLAB_NAN
                    } else if lo == hi {
                        if a.is_finite() && b.is_finite() {
                            a
                        } else {
                            MATLAB_NAN
                        }
                    } else {
                        a + (y - lo) / (hi - lo) * (b - a)
                    };
                }
                l += 1;
                return Ok(());
            }
            let c = match &center {
                Center::None => 0.,
                Center::Mean => mean(&v),
                Center::Median => median(&v),
                Center::Value(g) => g.at(l),
            };
            let s = match &scale {
                Scale::None => 1.,
                Scale::Std => std_dev(&v, false),
                Scale::Mad => median_abs_dev(&v),
                Scale::First => v.first().copied().unwrap_or(MATLAB_NAN),
                Scale::Iqr => {
                    let mut s = v.clone();
                    s.sort_unstable_by(f64::total_cmp);
                    if s.is_empty() {
                        MATLAB_NAN
                    } else {
                        iqr_sorted(&s)
                    }
                }
                Scale::Norm(p) => p_norm(&v, *p),
                Scale::Value(g) => g.at(l),
            };
            for (o, &y) in out.iter_mut().zip(lane) {
                *o = (y - c) / s;
            }
            l += 1;
            Ok(())
        },
    )?))
}

/// Vector p-norm with scaling by the largest magnitude against overflow.
fn p_norm(v: &[f64], p: f64) -> f64 {
    let big = v.iter().fold(0f64, |m, x| m.max(x.abs()));
    if p == f64::INFINITY || big == 0. || !big.is_finite() {
        return big;
    }
    let sum: f64 = v.iter().map(|x| (x.abs() / big).powf(p)).sum();
    big * sum.powf(1. / p)
}

// ------------------------------------------------------------------ rescale

/// A bound broadcast against an r x c matrix: scalar, row, column or full.
struct Bound {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl Bound {
    fn scalar(v: f64) -> Self {
        Self {
            rows: 1,
            cols: 1,
            data: vec![v],
        }
    }
    fn of(value: &Value, what: &str, rows: usize, cols: usize) -> Result<Self> {
        let Some(m) = numeric(value) else {
            return Err(format!("rescale {what} must be numeric"));
        };
        let ok = |n: usize, full: usize| n == 1 || n == full;
        if m.is_nd() || !ok(m.rows(), rows) || !ok(m.cols(), cols) || m.is_empty() {
            return Err(format!(
                "rescale {what} must be a scalar or have sizes compatible with the input"
            ));
        }
        Ok(Self {
            rows: m.rows(),
            cols: m.cols(),
            data: m.data().to_vec(),
        })
    }
    fn at(&self, i: usize, j: usize) -> f64 {
        let i = if self.rows == 1 { 0 } else { i };
        let j = if self.cols == 1 { 0 } else { j };
        self.data[i + j * self.rows]
    }
}

fn rescale(e: &Engine, args: &[Value]) -> Result<Value> {
    let usage = "Use rescale(A), rescale(A, l, u) and the options 'InputMin' and 'InputMax'";
    let x = real_data(e, &args[0], "rescale")?;
    let (rows, cols) = (x.rows(), x.cols());
    let mut at = 1;
    let mut lower = Bound::scalar(0.);
    let mut upper = Bound::scalar(1.);
    if matches!(args.get(1), Some(Value::Matrix(_))) {
        let Some(u) = args.get(2).filter(|v| matches!(v, Value::Matrix(_))) else {
            return Err(usage.into());
        };
        lower = Bound::of(&args[1], "lower bound", rows, cols)?;
        upper = Bound::of(u, "upper bound", rows, cols)?;
        at = 3;
    }
    let data = x.data();
    let lo_all = data
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .fold(f64::INFINITY, f64::min);
    let hi_all = data
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .fold(f64::NEG_INFINITY, f64::max);
    let mut in_min = Bound::scalar(lo_all);
    let mut in_max = Bound::scalar(hi_all);
    while at < args.len() {
        let name = text(&args[at]).ok_or(usage)?;
        let value = args
            .get(at + 1)
            .ok_or_else(|| format!("rescale option '{name}' needs a value"))?;
        match name.as_str() {
            "inputmin" => in_min = Bound::of(value, "InputMin", rows, cols)?,
            "inputmax" => in_max = Bound::of(value, "InputMax", rows, cols)?,
            _ => return Err(usage.into()),
        }
        at += 2;
    }
    budget(e, x.len())?;
    let mut out = Vec::with_capacity(x.len());
    for j in 0..cols {
        for i in 0..rows {
            poll(e, out.len())?;
            let (a, b) = (in_min.at(i, j), in_max.at(i, j));
            let (l, u) = (lower.at(i, j), upper.at(i, j));
            let y = data[i + j * rows];
            out.push(if y.is_nan() || a.is_nan() || b.is_nan() {
                // A NaN input bound has no interval to map from.
                MATLAB_NAN
            } else if a == b {
                if a.is_finite() && l.is_finite() && u.is_finite() {
                    l
                } else {
                    MATLAB_NAN
                }
            } else {
                let y = y.clamp(a.min(b), a.max(b));
                l + (y - a) / (b - a) * (u - l)
            });
        }
    }
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, out)?))
}

// ------------------------------------------------------------------ bounds

fn bounds(e: &mut Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    let usage = "Use [S, L] = bounds(A[, dim][, 'omitnan'|'includenan'])";
    if args.len() > 3 {
        return Err(usage.into());
    }
    let mut include_nan = false;
    let mut dim = None;
    for v in &args[1..] {
        match text(v).as_deref() {
            Some("omitnan" | "omitmissing") => include_nan = false,
            Some("includenan" | "includemissing") => include_nan = true,
            Some("all") => {
                return Err("bounds(A, 'all') is not supported yet; use bounds(A(:))".into());
            }
            Some(_) => return Err(usage.into()),
            None if dim.is_none() => dim = Some(dimension(v, "bounds")?),
            None => return Err(usage.into()),
        }
    }
    if matches!(args[0], Value::Complex(_)) {
        return Err("bounds of complex input is not supported yet".into());
    }
    if args[0].is_nd() {
        return Err("bounds of arrays with more than two dimensions is not supported yet".into());
    }
    // Reuse min/max for shapes, classes and empty semantics (NaN is omitted there).
    let empty = Value::Matrix(Matrix::filled(0, 0, 0.)?);
    let mut call = vec![args[0].clone()];
    if let Some(d) = dim {
        call.push(empty);
        call.push(Value::Matrix(Matrix::scalar(d as f64)));
    }
    // Integer data (the dispatcher passes only those as typed values) uses the
    // native min/max selection, so every value keeps its class exactly.
    let mut extreme = |which: &str| -> Result<Value> {
        if typed_class(&args[0]).is_some() {
            e.typed_extreme(which, &call, 1, false)?
                .map(|mut v| v.remove(0))
                .ok_or_else(|| format!("bounds: {which} of the integer input failed"))
        } else {
            Ok(e.reduction_call(which, &call, 1, [true, false, false])?
                .remove(0))
        }
    };
    let mut lo = extreme("min")?;
    let mut hi = if count > 1 {
        extreme("max")?
    } else {
        lo.clone()
    };
    if include_nan && let Value::Matrix(x) = &args[0] {
        let d = dim.unwrap_or_else(|| default_dim(x));
        let nan = map_lanes(e, x, d, 1, |lane, out| {
            out[0] = if lane.iter().any(|v| v.is_nan()) {
                1.
            } else {
                0.
            };
            Ok(())
        })?;
        for target in [&mut lo, &mut hi] {
            if let Value::Matrix(m) = target
                && m.len() == nan.len()
            {
                let data: Vec<f64> = m
                    .data()
                    .iter()
                    .zip(nan.data())
                    .map(|(&v, &f)| if f == 1. { MATLAB_NAN } else { v })
                    .collect();
                *m = Matrix::from_column_vec(m.rows(), m.cols(), data)?;
            }
        }
    }
    Ok(vec![lo, hi])
}

// --------------------------------------------------------------- smoothdata

#[derive(Clone, Copy, PartialEq)]
enum Method {
    MovMean,
    MovMedian,
    Gaussian,
    Lowess,
    Loess,
    RLowess,
    RLoess,
    Sgolay,
}

fn method_of(name: &str) -> Option<Method> {
    Some(match name {
        "movmean" => Method::MovMean,
        "movmedian" => Method::MovMedian,
        "gaussian" => Method::Gaussian,
        "lowess" => Method::Lowess,
        "loess" => Method::Loess,
        "rlowess" => Method::RLowess,
        "rloess" => Method::RLoess,
        "sgolay" => Method::Sgolay,
        _ => return None,
    })
}

/// (before, after) of a scalar window k: centered, one extra sample behind for even k.
fn window_split(k: usize) -> (usize, usize) {
    (k / 2, (k - 1) / 2)
}

fn smoothdata(e: &mut Engine, args: &[Value]) -> Result<Vec<Value>> {
    let usage = "Use smoothdata(A[, dim][, method][, window][, nanflag], Name, Value) with method 'movmean', 'movmedian', 'gaussian', 'lowess', 'loess', 'rlowess', 'rloess' or 'sgolay'";
    let x = real_data(e, &args[0], "smoothdata")?;
    let mut dim = default_dim(&x);
    let mut method = None;
    let mut window: Option<Matrix> = None;
    let mut omit_nan = true;
    let mut factor = None;
    let mut degree = None;
    let mut at = 1;
    while at < args.len() {
        match (&args[at], text(&args[at])) {
            (Value::Matrix(_), _) if method.is_none() && at == 1 => {
                dim = dimension(&args[at], "smoothdata")?;
            }
            (Value::Matrix(m), _) if window.is_none() && method.is_some() => {
                window = Some(m.clone());
            }
            (_, Some(t)) if method.is_none() && method_of(&t).is_some() => method = method_of(&t),
            (_, Some(t)) if t == "omitnan" || t == "omitmissing" => omit_nan = true,
            (_, Some(t)) if t == "includenan" || t == "includemissing" => omit_nan = false,
            (_, Some(t)) if t == "smoothingfactor" || t == "degree" || t == "samplepoints" => {
                let value = args
                    .get(at + 1)
                    .and_then(numeric)
                    .filter(|m| m.len() == 1)
                    .map(|m| m.data()[0]);
                match t.as_str() {
                    "samplepoints" => {
                        return Err("smoothdata 'SamplePoints' is not supported yet".into());
                    }
                    "smoothingfactor" => match value {
                        Some(v) if (0. ..=1.).contains(&v) => factor = Some(v),
                        _ => {
                            return Err(
                                "smoothdata 'SmoothingFactor' must be a scalar between 0 and 1"
                                    .into(),
                            );
                        }
                    },
                    _ => match value {
                        Some(v) if v >= 0. && v.fract() == 0. && v <= 1e6 => {
                            degree = Some(v as usize)
                        }
                        _ => {
                            return Err(
                                "smoothdata 'Degree' must be a nonnegative integer scalar".into()
                            );
                        }
                    },
                }
                at += 1;
            }
            _ => return Err(usage.into()),
        }
        at += 1;
    }
    if degree.is_some() && method != Some(Method::Sgolay) {
        return Err("smoothdata 'Degree' applies only to the 'sgolay' method".into());
    }
    if factor.is_some() && window.is_some() {
        return Err("smoothdata 'SmoothingFactor' cannot be combined with a window".into());
    }
    let (before, after, window_value) = match &window {
        Some(w) => {
            let integer = |v: f64| v.is_finite() && v >= 0. && v.fract() == 0. && v <= 1e9;
            match w.data() {
                [k] if integer(*k) && *k >= 1. => {
                    let (b, a) = window_split(*k as usize);
                    (b, a, w.clone())
                }
                [b, a] if integer(*b) && integer(*a) => (*b as usize, *a as usize, w.clone()),
                _ => {
                    return Err("smoothdata window must be a positive integer scalar or two nonnegative integers [kb kf]; non-integer windows are not supported yet".into());
                }
            }
        }
        None => {
            let mut k = default_window(e, &x, dim, factor.unwrap_or(DEFAULT_FACTOR))?;
            if method == Some(Method::Sgolay) {
                // The fitted polynomial needs more samples than its degree.
                k = k.max(degree.unwrap_or(2) + 1);
            }
            let (b, a) = window_split(k);
            (b, a, Matrix::scalar(k as f64))
        }
    };
    let method = method.unwrap_or(Method::MovMean);
    let width = before.saturating_add(after).saturating_add(1);
    let degree = degree.unwrap_or(2);
    if method == Method::Sgolay && degree >= width {
        return Err(format!(
            "smoothdata 'sgolay' degree ({degree}) must be less than the window length ({width})"
        ));
    }
    let lanes = Lanes::new(&x, dim);
    let smoothed = match method {
        Method::MovMean | Method::MovMedian => {
            // The moving-statistics kernels of movmean and movmedian.
            let call = [
                Value::Matrix(x.clone()),
                Value::Matrix(Matrix::from_column_vec(
                    1,
                    2,
                    vec![before as f64, after as f64],
                )?),
                Value::Matrix(Matrix::scalar(dim as f64)),
                Value::Text(if omit_nan { "omitnan" } else { "includenan" }.into()),
            ];
            if dim > 2 {
                Value::Matrix(x.clone())
            } else if method == Method::MovMean {
                e.statsx_outputs("movmean", &call, 1)?.remove(0)
            } else {
                e.arx_outputs("movmedian", &call, 1)?.remove(0)
            }
        }
        _ => {
            let fit_degree = match method {
                Method::Gaussian => 0,
                Method::Lowess | Method::RLowess => 1,
                Method::Loess | Method::RLoess => 2,
                _ => degree,
            };
            let points = (lanes.count() as u128) * (lanes.len() as u128);
            let per_point =
                (width.min(lanes.len().max(1)) as u128) * ((fit_degree + 1) as u128).pow(2);
            let passes = if matches!(method, Method::RLowess | Method::RLoess) {
                ROBUST_PASSES as u128 + 1
            } else {
                1
            };
            if points * per_point * passes > FIT_WORK {
                return Err("smoothdata: the window is too large for this much data; use a smaller window or less data".into());
            }
            let fit = LocalFit {
                before,
                after,
                degree: fit_degree,
                kind: match method {
                    Method::Gaussian => Kernel::Gaussian,
                    Method::Sgolay => Kernel::Uniform,
                    _ => Kernel::Tricube,
                },
                omit_nan,
            };
            let robust = matches!(method, Method::RLowess | Method::RLoess);
            Value::Matrix(map_lanes(e, &x, dim, lanes.len(), |lane, out| {
                fit.smooth(e, lane, robust, out)
            })?)
        }
    };
    Ok(vec![smoothed, Value::Matrix(window_value)])
}

/// Default window: the smallest moving-average length k whose response
/// |H_k(f)| = |sin(pi f k) / (k sin(pi f))| removes at least the fraction
/// `factor` of the data's energy (n when no k does). The energy is the
/// periodogram of each mean-removed lane (NaN replaced by the lane mean),
/// pooled over lanes. Constant data gives 1.
///
/// The removed fraction is not monotonic in k (alternating data loses all its
/// energy at k = 2 but not at k = 3), so every k is tried in increasing order.
/// Each try is O(1): |H_k|^2 is the transform of the triangle (k - |m|)/k^2,
/// so the kept energy sum_j p_j |H_k(f_j)|^2 is (k R(0) + 2 T(k)) / k^2 with
/// the circular autocorrelation R(m) = sum_j p_j cos(2 pi j m / n) of the
/// pooled periodogram p and T(k) = sum_{m<k} (k - m) R(m) = T(k-1) + sum_{m<k} R(m).
fn default_window(e: &Engine, x: &Matrix, dim: usize, factor: f64) -> Result<usize> {
    let lanes = Lanes::new(x, dim);
    let n = lanes.len();
    if n <= 1 || factor == 0. {
        return Ok(1);
    }
    budget(e, n)?;
    let mut planner = rustfft::FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    let mut power = vec![0.; n];
    let mut lane = Vec::with_capacity(n);
    let mut buffer = vec![rustfft::num_complex::Complex::new(0., 0.); n];
    for l in 0..lanes.count() {
        poll(e, l)?;
        lanes.lane(x.data(), l, &mut lane);
        let m = mean(&present(&lane));
        if m.is_nan() {
            continue;
        }
        for (b, &v) in buffer.iter_mut().zip(&lane) {
            *b = rustfft::num_complex::Complex::new(if v.is_nan() { 0. } else { v - m }, 0.);
        }
        fft.process(&mut buffer);
        for (p, b) in power.iter_mut().zip(&buffer) {
            *p += b.norm_sqr();
        }
    }
    let total: f64 = power[1..].iter().sum();
    if total.is_nan() || total <= 0. || total.is_infinite() {
        return Ok(1);
    }
    // R(m): the periodogram is real and even, so its transform is real.
    for (b, &p) in buffer.iter_mut().zip(&power) {
        *b = rustfft::num_complex::Complex::new(p, 0.);
    }
    fft.process(&mut buffer);
    let r0 = buffer[0].re;
    // k = 1 keeps all the energy and removes nothing (factor > 0 here).
    let (mut s, mut t) = (0., 0.);
    for k in 2..=n {
        poll(e, k)?;
        s += buffer[k - 1].re;
        t += s;
        let kf = k as f64;
        // The j = 0 term (|H_k(0)| = 1) is not part of the energy.
        let kept = (kf * r0 + 2. * t) / (kf * kf) - power[0];
        if (total - kept) / total >= factor {
            return Ok(k);
        }
    }
    Ok(n)
}

#[derive(Clone, Copy, PartialEq)]
enum Kernel {
    Uniform,
    Tricube,
    Gaussian,
}

/// Local polynomial fit over the window [i - before, i + after], clipped to
/// the lane (windows shrink at the ends).
struct LocalFit {
    before: usize,
    after: usize,
    degree: usize,
    kind: Kernel,
    omit_nan: bool,
}

impl LocalFit {
    fn smooth(&self, e: &Engine, lane: &[f64], robust: bool, out: &mut [f64]) -> Result<()> {
        let n = lane.len();
        if n == 0 {
            return Ok(());
        }
        self.pass(e, lane, None, out)?;
        if !robust {
            return Ok(());
        }
        let mut weights = vec![1.; n];
        // Residuals below this are rounding noise of an exact fit and count as 0.
        let tiny = 1e-12
            * lane
                .iter()
                .filter(|v| !v.is_nan())
                .fold(0f64, |m, v| m.max(v.abs()));
        for _ in 0..ROBUST_PASSES {
            let residuals: Vec<f64> = lane
                .iter()
                .zip(out.iter())
                .filter(|(y, f)| !y.is_nan() && !f.is_nan())
                .map(|(y, f)| (y - f).abs())
                .map(|r| if r <= tiny { 0. } else { r })
                .collect();
            let m = median(&residuals);
            if m.is_nan() || m <= 0. {
                break;
            }
            for ((w, y), f) in weights.iter_mut().zip(lane).zip(out.iter()) {
                let u = (y - f) / (6. * m);
                if u.is_nan() {
                    // No residual (a NaN fit, e.g. its window holds a NaN under
                    // 'includenan'): the sample keeps its weight, so it is left
                    // out of the reweighting as it is left out of the median.
                    continue;
                }
                *w = if u.abs() < 1. {
                    (1. - u * u) * (1. - u * u)
                } else {
                    0.
                };
            }
            self.pass(e, lane, Some(&weights), out)?;
        }
        Ok(())
    }

    fn pass(
        &self,
        e: &Engine,
        lane: &[f64],
        robust: Option<&[f64]>,
        out: &mut [f64],
    ) -> Result<()> {
        let n = lane.len();
        // Interior windows without NaN share one coefficient vector.
        let mut interior: Option<Vec<f64>> = None;
        let mut t = Vec::new();
        let mut y = Vec::new();
        let mut w = Vec::new();
        for (i, slot) in out.iter_mut().enumerate() {
            poll(e, i)?;
            let lo = i.saturating_sub(self.before);
            let hi = i.saturating_add(self.after).min(n - 1);
            let window = &lane[lo..=hi];
            let has_nan = window.iter().any(|v| v.is_nan());
            if has_nan && !self.omit_nan {
                *slot = MATLAB_NAN;
                continue;
            }
            let full =
                robust.is_none() && !has_nan && lo + self.before == i && hi == i + self.after;
            if full && let Some(h) = &interior {
                *slot = h.iter().zip(window).map(|(c, v)| c * v).sum();
                continue;
            }
            t.clear();
            y.clear();
            w.clear();
            for (j, &v) in window.iter().enumerate() {
                if v.is_nan() {
                    continue;
                }
                t.push((lo + j) as f64 - i as f64);
                y.push(v);
                w.push(robust.map_or(1., |r| r[lo + j]));
            }
            let base = self.kernel_weights(&t);
            for (wi, b) in w.iter_mut().zip(&base) {
                *wi *= b;
            }
            let Some(h) = coefficients(&t, &w, self.degree) else {
                *slot = MATLAB_NAN;
                continue;
            };
            *slot = h.iter().zip(&y).map(|(c, v)| c * v).sum();
            if full {
                interior = Some(h);
            }
        }
        Ok(())
    }

    fn kernel_weights(&self, t: &[f64]) -> Vec<f64> {
        match self.kind {
            Kernel::Uniform => vec![1.; t.len()],
            Kernel::Tricube => {
                // Distances scaled by one more than the widest offset, so
                // every sample of the window keeps a positive weight.
                let d = t.iter().fold(0f64, |m, v| m.max(v.abs())) + 1.;
                t.iter()
                    .map(|v| {
                        let u = (v / d).abs();
                        let c = 1. - u * u * u;
                        c * c * c
                    })
                    .collect()
            }
            Kernel::Gaussian => {
                let sigma = (self.before + self.after + 1) as f64 / 5.;
                t.iter()
                    .map(|v| (-0.5 * (v / sigma) * (v / sigma)).exp())
                    .collect()
            }
        }
    }
}

/// Coefficients h with fit(0) = sum h_j y_j for the weighted least-squares
/// polynomial of the given degree in offsets t (degree lowered when fewer
/// samples carry weight). Solved by Householder QR of the weighted
/// Vandermonde matrix in offsets scaled to [-1, 1].
fn coefficients(t: &[f64], w: &[f64], degree: usize) -> Option<Vec<f64>> {
    let used: Vec<usize> = (0..t.len()).filter(|&j| w[j] > 0.).collect();
    if used.is_empty() {
        return None;
    }
    let degree = degree.min(used.len() - 1);
    let mut h = vec![0.; t.len()];
    if degree == 0 {
        let total: f64 = used.iter().map(|&j| w[j]).sum();
        for &j in &used {
            h[j] = w[j] / total;
        }
        return Some(h);
    }
    let scale = t.iter().fold(0f64, |m, v| m.max(v.abs())).max(1.);
    let m = used.len();
    let p = degree + 1;
    let a = DMatrix::from_fn(m, p, |r, c| {
        let j = used[r];
        w[j].sqrt() * (t[j] / scale).powi(c as i32)
    });
    let qr = a.qr();
    let r = qr.r();
    // fit(0) = c_0 = e_0' R^-1 Q' sqrt(W) y, so h = sqrt(W) Q R^-T e_0.
    let mut e0 = DVector::zeros(p);
    e0[0] = 1.;
    let g = r.transpose().solve_lower_triangular(&e0)?;
    let q = qr.q();
    let hq = q * g;
    for (r, &j) in used.iter().enumerate() {
        h[j] = w[j].sqrt() * hq[r];
    }
    Some(h)
}
