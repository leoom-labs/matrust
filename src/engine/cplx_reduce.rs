//! Complex input for sum, prod, mean, cumsum, cumprod, diff, std, var, median,
//! mode, sort, sortrows, unique, round, floor, ceil, fix, flip, fliplr, flipud,
//! circshift and nnz, plus the reference argument checks of reshape, cross, dot
//! and min/max for complex input (docs/tasks/2026-10-25-complex-corpora-report.md).
//! Complex values order by abs, then by angle in (-pi, pi], NaN last; every
//! ordering here goes through `Key`. Sums, means, cumulative sums and
//! differences run the real kernels on the real and imaginary parts, so they
//! keep the real path's summation order. Private methods use the `cxr_` prefix.
use super::{Engine, Execution, Value};
use crate::matrix::Dispersion;
use crate::{Array, Complex64 as C, ComplexMatrix, Matrix, Result, matrix::MAX_ELEMENTS};
use std::cmp::Ordering;

const NAMES: &[&str] = &[
    "sum",
    "prod",
    "mean",
    "cumsum",
    "cumprod",
    "diff",
    "std",
    "var",
    "median",
    "round",
    "floor",
    "ceil",
    "fix",
    "flip",
    "fliplr",
    "flipud",
    "circshift",
    "reshape",
    "nnz",
    "cross",
    "dot",
];

const DIM_ERROR: &str =
    "Dimension argument must be a positive integer scalar within indexing range.";
const TOO_MANY: &str = "Too many input arguments.";
/// Items per cancellation poll and per std-sorted run of the merge sort.
const RUN: usize = 4096;
/// Nominal row x key x comparison-level bound of sortrows (the real path's).
const MAX_SORT_WORK: usize = 512 * 1024 * 1024;
const CIRCSHIFT_WARNING: &str = "CIRCSHIFT(X,K) with scalar K and where size(X,1)==1 will change behavior in future versions. To\nretain current behavior, use CIRCSHIFT(X,[K,0]) instead.";

/// Sort key of one complex value: NaN last, then abs, then angle in (-pi, pi].
#[derive(Clone, Copy)]
struct Key {
    nan: bool,
    abs: f64,
    angle: f64,
}

impl Key {
    fn of(z: C) -> Self {
        let angle = libm::atan2(z.im, z.re);
        Self {
            nan: z.re.is_nan() || z.im.is_nan(),
            abs: libm::hypot(z.re, z.im),
            angle: if angle == -std::f64::consts::PI {
                std::f64::consts::PI
            } else {
                angle
            },
        }
    }
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.nan, other.nan) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => self
                .abs
                .partial_cmp(&other.abs)
                .unwrap_or(Ordering::Equal)
                .then_with(|| {
                    self.angle
                        .partial_cmp(&other.angle)
                        .unwrap_or(Ordering::Equal)
                }),
        }
    }
}

/// Lanes of a rows x cols array along `dim`: (count, length, lane stride,
/// element stride). Dimensions above 2 give one lane per element.
fn lanes(rows: usize, cols: usize, dim: usize) -> (usize, usize, usize, usize) {
    match dim {
        1 => (cols, rows, rows, 1),
        2 => (rows, cols, 1, rows),
        _ => (rows * cols, 1, 1, 1),
    }
}

/// Result shape of a reduction along `dim`.
fn reduced(rows: usize, cols: usize, dim: usize) -> (usize, usize) {
    match dim {
        1 => (1, cols),
        2 => (rows, 1),
        _ => (rows, cols),
    }
}

/// MATLAB's default working dimension: the first one whose size is not 1.
fn first_dim(rows: usize, cols: usize) -> usize {
    if rows == 1 && cols != 1 { 2 } else { 1 }
}

fn dim_of(value: &Value) -> Result<usize> {
    let d = match value {
        Value::Matrix(m) if m.len() == 1 => m.data()[0],
        _ => f64::NAN,
    };
    if d >= 1. && d.fract() == 0. && d <= MAX_ELEMENTS as f64 {
        Ok(d as usize)
    } else {
        Err(DIM_ERROR.into())
    }
}

fn complex_arg(value: &Value) -> &ComplexMatrix {
    match value {
        Value::Complex(z) => z,
        _ => unreachable!("routed only for complex input"),
    }
}

fn shape(value: &Value) -> Option<(usize, usize)> {
    match value {
        Value::Matrix(m) => Some((m.rows(), m.cols())),
        Value::Logical(m) => Some((m.rows(), m.cols())),
        Value::Complex(z) => Some((z.rows(), z.cols())),
        _ => None,
    }
}

/// `x./abs(x)` for a complex value, 0 for 0 (MATLAB sign).
fn sign(z: C) -> C {
    let r = libm::hypot(z.re, z.im);
    if r == 0. {
        C::new(0., 0.)
    } else {
        C::new(z.re / r, z.im / r)
    }
}

/// Midpoint: A + (B-A)/2, or (A+B)/2 when the signs
/// differ or either value is infinite.
fn mean_of(a: C, b: C) -> C {
    let infinite = |z: C| z.re.is_infinite() || z.im.is_infinite();
    if sign(a) != sign(b) || infinite(a) || infinite(b) {
        let s = a + b;
        C::new(s.re / 2., s.im / 2.)
    } else {
        let d = b - a;
        C::new(a.re + d.re / 2., a.im + d.im / 2.)
    }
}

/// round(x, n) with n decimal digits (negative n rounds left of the point).
fn round_digits(x: f64, n: i32) -> f64 {
    if !x.is_finite() {
        return x;
    }
    if n >= 0 {
        let scale = 10f64.powi(n);
        let y = x * scale;
        if !y.is_finite() || y.abs() >= 4503599627370496. {
            x
        } else {
            y.round() / scale
        }
    } else {
        let scale = 10f64.powi(-n);
        if scale.is_finite() {
            (x / scale).round() * scale
        } else {
            0f64.copysign(x)
        }
    }
}

/// Elements of complex std/var input above which lanes run on the worker pool
/// (the real dispersion kernel's threshold).
const PAR_DISPERSION: usize = 512 * 1024;

/// One lane of complex var/std by the corrected two-pass algorithm of Chan,
/// Golub & LeVeque, "Algorithms for computing the sample variance: analysis and
/// recommendations", Amer. Statistician 37 (1983), sec. 2, applied to complex
/// deviations: with `m = sum(z)/n`,
/// `S = sum |z_i - m|^2 - |sum (z_i - m)|^2 / n`, where the second term removes
/// the rounding error of `m`. `|z_i - m|` is a hypot, as `abs` gives it.
/// Values are first divided by a power of two near the largest component (exact),
/// so the sum cannot overflow; the deviations `z_i - m` are then divided by a
/// power of two near their own largest component, so their squares neither
/// overflow nor underflow (`var([1e200+1i, 1e200-1i])` is 2, not 0). Components
/// below 2^-1022 of the largest one are lost to the first division. NaNs are
/// skipped when `omit_nan`; otherwise NaN or Inf gives NaN. One value or all
/// zeros (or equal values) give 0.
fn dispersion_lane(
    at: impl Fn(usize) -> C,
    len: usize,
    population: bool,
    omit_nan: bool,
    kind: Dispersion,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<f64> {
    let poll = |k: usize| -> Result<()> {
        if k.is_multiple_of(RUN) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Calculation cancelled".into());
        }
        Ok(())
    };
    let skipped = |z: C| omit_nan && (z.re.is_nan() || z.im.is_nan());
    let (mut n, mut finite, mut largest) = (0usize, true, 0f64);
    for k in 0..len {
        poll(k)?;
        let z = at(k);
        if skipped(z) {
            continue;
        }
        n += 1;
        finite &= z.re.is_finite() && z.im.is_finite();
        largest = largest.max(z.re.abs()).max(z.im.abs());
    }
    if n == 0 || !finite {
        return Ok(f64::NAN);
    }
    if n == 1 || largest == 0. {
        return Ok(0.);
    }
    let exponent = libm::frexp(largest).1 - 1;
    let scale = libm::scalbn(1., exponent);
    let scaled = |z: C| C::new(z.re / scale, z.im / scale);
    let mut sum = C::new(0., 0.);
    for k in 0..len {
        poll(k)?;
        let z = at(k);
        if !skipped(z) {
            sum += scaled(z);
        }
    }
    let mean = C::new(sum.re / n as f64, sum.im / n as f64);
    let mut widest = 0f64;
    for k in 0..len {
        poll(k)?;
        let z = at(k);
        if !skipped(z) {
            let d = scaled(z) - mean;
            widest = widest.max(d.re.abs()).max(d.im.abs());
        }
    }
    if widest == 0. {
        return Ok(0.);
    }
    let spread_exponent = libm::frexp(widest).1 - 1;
    let spread_scale = libm::scalbn(1., spread_exponent);
    let (mut squares, mut residual) = (0f64, C::new(0., 0.));
    for k in 0..len {
        poll(k)?;
        let z = at(k);
        if !skipped(z) {
            let d = scaled(z) - mean;
            let d = C::new(d.re / spread_scale, d.im / spread_scale);
            let size = libm::hypot(d.re, d.im);
            squares += size * size;
            residual += d;
        }
    }
    let correction = (residual.re * residual.re + residual.im * residual.im) / n as f64;
    let spread = (squares - correction).max(0.);
    let normalized = spread / if population { n } else { n - 1 } as f64;
    let exponent = exponent + spread_exponent;
    Ok(match kind {
        Dispersion::Variance => libm::scalbn(normalized, 2 * exponent),
        Dispersion::StandardDeviation => libm::scalbn(normalized.sqrt(), exponent),
    })
}

impl Engine {
    /// Single-output entry for `fn call`: None unless `name` is a cplx2
    /// function and its data argument is complex (either one for dot/cross).
    pub(super) fn cxr_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        let complex = |i: usize| matches!(args.get(i), Some(Value::Complex(_)));
        if !(complex(0) || (matches!(name, "dot" | "cross") && complex(1))) {
            return Ok(None);
        }
        self.check_cancel()?;
        let value = match name {
            "sum" | "mean" => self.cxr_sum(name, args)?,
            "prod" => {
                if args.len() > 2 {
                    return Err(TOO_MANY.into());
                }
                if let Some(d) = args.get(1) {
                    dim_of(d)?;
                }
                let value = self.array_function("prod", args)?;
                self.cxr_narrow_value(value)?
            }
            "cumsum" | "cumprod" => self.cxr_cumulative(name, args)?,
            "diff" => self.cxr_diff(args)?,
            "std" | "var" => self.cxr_dispersion(name, args)?,
            "median" => self.cxr_median(args)?,
            "round" | "floor" | "ceil" | "fix" => self.cxr_round(name, args)?,
            "flip" | "fliplr" | "flipud" => self.cxr_flip(name, args)?,
            "circshift" => {
                let warn = args.len() == 2
                    && complex_arg(&args[0]).rows() == 1
                    && matches!(&args[1], Value::Matrix(k) if k.len() == 1);
                let value = self.array_function("circshift", args)?;
                if warn {
                    self.lang_utils_call("warning", &[Value::Text(CIRCSHIFT_WARNING.into())], out)?;
                }
                value
            }
            "nnz" => {
                if args.len() > 1 {
                    return Err(TOO_MANY.into());
                }
                let z = complex_arg(&args[0]);
                let mut count = 0usize;
                for chunk in z.data().chunks(8192) {
                    self.check_cancel()?;
                    count += chunk.iter().filter(|v| v.re != 0. || v.im != 0.).count();
                }
                Value::Matrix(Matrix::scalar(count as f64))
            }
            "reshape" => {
                self.cxr_reshape_check(args)?;
                return Ok(None);
            }
            "cross" => {
                if let [a, b] = args
                    && let (Some(a), Some(b)) = (shape(a), shape(b))
                {
                    let vector = |(r, c): (usize, usize)| r == 1 || c == 1;
                    if vector(a) && vector(b) && (a.0 * a.1 != 3 || b.0 * b.1 != 3) {
                        return Err("A and B must be of length 3 in the dimension in which the cross product is taken.".into());
                    }
                }
                return Ok(None);
            }
            _ => {
                if let [a, b, ..] = args
                    && let (Some(a), Some(b)) = (shape(a), shape(b))
                {
                    let vector = |(r, c): (usize, usize)| r == 1 || c == 1;
                    let same = if args.len() == 2 && vector(a) && vector(b) {
                        a.0 * a.1 == b.0 * b.1
                    } else {
                        a == b
                    };
                    if !same {
                        return Err("A and B must be same size.".into());
                    }
                }
                let value = self.array_function("dot", args)?;
                self.cxr_narrow_value(value)?
            }
        };
        self.check_cancel()?;
        Ok(Some(value))
    }

    fn cxr_narrow_value(&self, value: Value) -> Result<Value> {
        match value {
            Value::Complex(z) => self.narrowed(z),
            other => Ok(other),
        }
    }

    fn cxr_parts(&self, z: &ComplexMatrix) -> Result<(Matrix, Matrix)> {
        Ok((z.real(&self.cancel)?, z.imag(&self.cancel)?))
    }

    fn cxr_build(&self, rows: usize, cols: usize, data: Vec<C>) -> Result<ComplexMatrix> {
        ComplexMatrix::from_column_vec(rows, cols, data, &self.cancel)
    }

    /// sum(A[,dim]) narrows to real; mean is that sum divided by the count,
    /// so mean(complex([])) is a real NaN.
    fn cxr_sum(&self, name: &str, args: &[Value]) -> Result<Value> {
        let dim = match args {
            [_] => None,
            [_, d] => Some(dim_of(d)?),
            _ => {
                return Err(format!(
                    "{name}: complex input supports {name}(A) and {name}(A,dim) only"
                ));
            }
        };
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        let (re, im) = self.cxr_parts(z)?;
        let sum = ComplexMatrix::from_parts(
            &re.sum_along(dim, false, &self.cancel)?,
            &im.sum_along(dim, false, &self.cancel)?,
            &self.cancel,
        )?;
        let sum = self.narrowed(sum)?;
        if name == "sum" {
            return Ok(sum);
        }
        let n = match dim.unwrap_or(first_dim(rows, cols)) {
            _ if dim.is_none() && rows == 0 && cols == 0 => 0,
            1 => rows,
            2 => cols,
            _ => 1,
        } as f64;
        Ok(match sum {
            Value::Matrix(m) => Value::Matrix(Matrix::from_column_vec(
                m.rows(),
                m.cols(),
                m.data().iter().map(|v| v / n).collect(),
            )?),
            Value::Complex(z) => Value::Complex(
                self.cxr_build(
                    z.rows(),
                    z.cols(),
                    z.data()
                        .iter()
                        .map(|v| C::new(v.re / n, v.im / n))
                        .collect(),
                )?,
            ),
            other => other,
        })
    }

    /// cumsum runs the real kernel on both parts; cumprod scans each lane in
    /// order. Both stay complex (reference: cumsum([1i -1i]) is not real).
    fn cxr_cumulative(&self, name: &str, args: &[Value]) -> Result<Value> {
        if args.len() > 2 {
            return Err(TOO_MANY.into());
        }
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        let dim = match args.get(1) {
            Some(d) => dim_of(d)?,
            None => first_dim(rows, cols),
        };
        if name == "cumsum" {
            let (re, im) = self.cxr_parts(z)?;
            return Ok(Value::Complex(ComplexMatrix::from_parts(
                &re.cumulative(dim, false, &self.cancel)?,
                &im.cumulative(dim, false, &self.cancel)?,
                &self.cancel,
            )?));
        }
        let mut data = z.data().to_vec();
        let (count, len, lane_step, step) = lanes(rows, cols, dim);
        for lane in 0..count {
            if lane % 1024 == 0 {
                self.check_cancel()?;
            }
            let start = lane * lane_step;
            for k in 1..len {
                let i = start + k * step;
                data[i] = data[i - step] * data[i];
            }
        }
        Ok(Value::Complex(self.cxr_build(rows, cols, data)?))
    }

    /// diff(A[,n[,dim]]) on both parts with the real kernel; narrows to real.
    fn cxr_diff(&self, args: &[Value]) -> Result<Value> {
        if args.len() > 3 {
            return Err(TOO_MANY.into());
        }
        let order = match args.get(1) {
            None => 1,
            Some(Value::Matrix(m)) if m.is_empty() => 1,
            Some(Value::Matrix(m))
                if m.len() == 1
                    && m.data()[0] >= 1.
                    && m.data()[0].fract() == 0.
                    && m.data()[0] <= MAX_ELEMENTS as f64 =>
            {
                m.data()[0] as usize
            }
            Some(_) => {
                return Err("Difference order N must be a positive integer scalar.".into());
            }
        };
        let z = complex_arg(&args[0]);
        let (re, im) = self.cxr_parts(z)?;
        let dim = match args.get(2) {
            Some(d) => dim_of(d)?,
            None => re.first_nonsingleton_dimension(),
        };
        self.narrowed(ComplexMatrix::from_parts(
            &re.difference(order, dim, &self.cancel)?,
            &im.difference(order, dim, &self.cancel)?,
            &self.cancel,
        )?)
    }

    /// std/var(A[, w[, dim|'all']][, nanflag]) for complex A: a real result per
    /// lane from `abs(z - mean).^2` (see `dispersion_lane`).
    fn cxr_dispersion(&self, name: &str, args: &[Value]) -> Result<Value> {
        let mut count = args.len();
        let mut omit_nan = false;
        if let Some(Value::Text(flag)) = args.last() {
            match flag.to_ascii_lowercase().as_str() {
                "omitnan" | "omitmissing" => {
                    omit_nan = true;
                    count -= 1;
                }
                "includenan" | "includemissing" => count -= 1,
                _ => {}
            }
        }
        if !(1..=3).contains(&count) {
            return Err(format!(
                "Use {name}(A[, 0|1|[] [, dim|'all']][, 'omitnan'|'includenan']); vector weights and vector dimensions are not implemented"
            ));
        }
        let population = match args.get(1).filter(|_| count >= 2) {
            None => false,
            Some(Value::Matrix(w)) if w.is_empty() => false,
            Some(Value::Matrix(w)) if w.len() == 1 && w.data()[0] == 0. => false,
            Some(Value::Matrix(w)) if w.len() == 1 && w.data()[0] == 1. => true,
            Some(Value::Matrix(w)) if w.len() > 1 => {
                return Err("Statistics vector weights are not implemented; use scalar normalization 0 or 1".into());
            }
            Some(_) => {
                return Err(
                    "W must be a vector of nonnegative weights, or a scalar 0 or 1.".into(),
                );
            }
        };
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        // (output rows, output cols, lane length, lane start step, element step)
        let (out_rows, out_cols, len, lane_step, step) = match args.get(2).filter(|_| count == 3) {
            Some(Value::Text(all)) if all.eq_ignore_ascii_case("all") => (1, 1, z.len(), 0, 1),
            None if rows == 0 && cols == 0 => (1, 1, 0, 0, 1),
            dim => {
                let dim = match dim {
                    Some(d) => dim_of(d)?,
                    None => first_dim(rows, cols),
                };
                let (_, len, lane_step, step) = lanes(rows, cols, dim);
                let (out_rows, out_cols) = reduced(rows, cols, dim);
                (out_rows, out_cols, len, lane_step, step)
            }
        };
        let kind = if name == "std" {
            Dispersion::StandardDeviation
        } else {
            Dispersion::Variance
        };
        let output_len = Matrix::checked_len(out_rows, out_cols)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(output_len)
            .map_err(|_| "Cannot allocate statistics output".to_owned())?;
        output.resize(output_len, 0.);
        let data = z.data();
        let cancel: &std::sync::atomic::AtomicBool = &self.cancel;
        let calculate = |(lane, out): (usize, &mut f64)| -> Result<()> {
            let start = lane * lane_step;
            let at = |k: usize| data[start + k * step];
            *out = dispersion_lane(at, len, population, omit_nan, kind, cancel)?;
            Ok(())
        };
        // Lanes are independent and each keeps its serial arithmetic, so the
        // result does not depend on the worker count.
        if data.len() >= PAR_DISPERSION
            && output_len > 1
            && crate::parallel::current_worker_count() > 1
        {
            use rayon::prelude::*;
            output.par_iter_mut().enumerate().try_for_each(calculate)?;
        } else {
            output.iter_mut().enumerate().try_for_each(calculate)?;
        }
        self.check_cancel()?;
        Ok(Value::Matrix(Matrix::from_column_vec(
            out_rows, out_cols, output,
        )?))
    }

    /// Cancellation poll for long loops: once every `RUN` items.
    fn cxr_poll(&self, i: usize) -> Result<()> {
        if i.is_multiple_of(RUN) {
            self.check_cancel()?;
        }
        Ok(())
    }

    /// Sort keys of `data`, polling for cancellation as they are built.
    fn cxr_keys(&self, data: &[C]) -> Result<Vec<Key>> {
        let mut keys = Vec::with_capacity(data.len());
        for (i, &v) in data.iter().enumerate() {
            self.cxr_poll(i)?;
            keys.push(Key::of(v));
        }
        Ok(keys)
    }

    /// Stable cancellable merge sort of `order`: std-sorted runs of `RUN`
    /// items, then linear merge passes that poll as they go (the shape of
    /// `arx_sort`); ties keep their input order like `sort_by`.
    fn cxr_sort_by(
        &self,
        order: &mut [usize],
        compare: impl Fn(usize, usize) -> Ordering,
    ) -> Result<()> {
        for run in order.chunks_mut(RUN) {
            self.check_cancel()?;
            run.sort_by(|&a, &b| compare(a, b));
        }
        if order.len() <= RUN {
            return Ok(());
        }
        let mut scratch = order.to_vec();
        let mut width = RUN;
        while width < order.len() {
            for start in (0..order.len()).step_by(width * 2) {
                let middle = (start + width).min(order.len());
                let end = (middle + width).min(order.len());
                let (mut left, mut right) = (start, middle);
                for (i, slot) in scratch.iter_mut().enumerate().take(end).skip(start) {
                    if i.is_multiple_of(RUN) {
                        #[cfg(test)]
                        tests::cancel_after_merge_progress(self);
                        self.check_cancel()?;
                    }
                    if left < middle
                        && (right == end || compare(order[left], order[right]) != Ordering::Greater)
                    {
                        *slot = order[left];
                        left += 1;
                    } else {
                        *slot = order[right];
                        right += 1;
                    }
                }
            }
            for (a, b) in order.chunks_mut(RUN).zip(scratch.chunks(RUN)) {
                self.check_cancel()?;
                a.copy_from_slice(b);
            }
            width *= 2;
        }
        Ok(())
    }

    /// The values of one lane, stably ordered by `Key` (with their positions).
    fn cxr_sorted_lane(
        &self,
        data: &[C],
        start: usize,
        len: usize,
        step: usize,
    ) -> Result<Vec<usize>> {
        let mut keys = Vec::with_capacity(len);
        for k in 0..len {
            self.cxr_poll(k)?;
            keys.push(Key::of(data[start + k * step]));
        }
        let mut order: Vec<usize> = (0..len).collect();
        self.cxr_sort_by(&mut order, |a, b| keys[a].cmp(&keys[b]))?;
        Ok(order)
    }

    /// median(A[,dim]): the middle of the complex order; NaN lanes give
    /// NaN. Narrows to real.
    fn cxr_median(&self, args: &[Value]) -> Result<Value> {
        if args.len() > 2 {
            return Err(TOO_MANY.into());
        }
        let dim = args.get(1).map(dim_of).transpose()?;
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        if dim.is_none() && rows == 0 && cols == 0 {
            return Ok(Value::Matrix(Matrix::scalar(f64::NAN)));
        }
        let dim = dim.unwrap_or(first_dim(rows, cols));
        let (count, len, lane_step, step) = lanes(rows, cols, dim);
        let data = z.data();
        let nan = C::new(f64::NAN, 0.);
        let mut values = Vec::with_capacity(count);
        for lane in 0..count {
            self.check_cancel()?;
            let start = lane * lane_step;
            let at = |k: usize| data[start + k * step];
            if len == 0 || (0..len).any(|k| at(k).re.is_nan() || at(k).im.is_nan()) {
                values.push(nan);
                continue;
            }
            let order = self.cxr_sorted_lane(data, start, len, step)?;
            let half = len / 2;
            let middle = at(order[half]);
            values.push(if len % 2 == 0 {
                mean_of(at(order[half - 1]), middle)
            } else {
                middle
            });
        }
        let (out_rows, out_cols) = reduced(rows, cols, dim);
        self.narrowed(self.cxr_build(out_rows, out_cols, values)?)
    }

    /// [M,F] = mode(A[,dim]): the most frequent value, the first in the complex
    /// order on ties. M narrows to real (the reference indexes the sorted data).
    pub(super) fn cxr_mode(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        if nout > 2 {
            return Err("mode: the third output is not supported for complex input".into());
        }
        if args.len() > 2 {
            return Err(TOO_MANY.into());
        }
        let dim = args.get(1).map(dim_of).transpose()?;
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        if dim.is_none() && rows == 0 && cols == 0 {
            return Ok(vec![
                Value::Matrix(Matrix::scalar(f64::NAN)),
                Value::Matrix(Matrix::scalar(0.)),
            ]);
        }
        let dim = dim.unwrap_or(first_dim(rows, cols));
        let (count, len, lane_step, step) = lanes(rows, cols, dim);
        let data = z.data();
        let mut modes = Vec::with_capacity(count);
        let mut frequencies = Vec::with_capacity(count);
        for lane in 0..count {
            self.check_cancel()?;
            let start = lane * lane_step;
            if len == 0 {
                modes.push(C::new(f64::NAN, 0.));
                frequencies.push(0.);
                continue;
            }
            let order = self.cxr_sorted_lane(data, start, len, step)?;
            let at = |k: usize| data[start + order[k] * step];
            let (mut best, mut best_count) = (at(0), 0);
            let mut k = 0;
            while k < len {
                self.check_cancel()?;
                let mut end = k + 1;
                while end < len && at(end) == at(k) {
                    self.cxr_poll(end)?;
                    end += 1;
                }
                if end - k > best_count {
                    (best, best_count) = (at(k), end - k);
                }
                k = end;
            }
            modes.push(best);
            frequencies.push(best_count as f64);
        }
        let (out_rows, out_cols) = reduced(rows, cols, dim);
        Ok(vec![
            self.narrowed(self.cxr_build(out_rows, out_cols, modes)?)?,
            Value::Matrix(Matrix::from_column_vec(out_rows, out_cols, frequencies)?),
        ])
    }

    /// [B,I] = sort(A[,dim][,'ascend'|'descend']) in the complex order; stable,
    /// NaN last ascending and first descending. B stays complex.
    pub(super) fn cxr_sort(
        &self,
        args: &[Value],
        want_indices: bool,
    ) -> Result<(Value, Option<Matrix>)> {
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        let mut dim = first_dim(rows, cols);
        let mut descend = false;
        let mut at = 1;
        if let Some(d @ Value::Matrix(_)) = args.get(at) {
            dim = dim_of(d)?;
            at += 1;
        }
        if let Some(value) = args.get(at) {
            let text = match value {
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)) => {
                    self.text_value(value.clone())?
                }
                _ => String::new(),
            };
            descend = match text.to_ascii_lowercase().as_str() {
                "ascend" => false,
                "descend" => true,
                _ => return Err("Sorting direction must be 'ascend' or 'descend'.".into()),
            };
            at += 1;
        }
        if at < args.len() {
            return Err(TOO_MANY.into());
        }
        let outputs = z
            .len()
            .checked_mul(2 + usize::from(want_indices))
            .and_then(|n| self.evaluated_elements.checked_add(n));
        if outputs.is_none_or(|n| n > 32 * 1024 * 1024) {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let data = z.data();
        let keys = self.cxr_keys(data)?;
        let mut sorted = data.to_vec();
        let mut indices = vec![1.; if want_indices { data.len() } else { 0 }];
        let (count, len, lane_step, step) = lanes(rows, cols, dim);
        let mut order: Vec<usize> = Vec::with_capacity(len);
        for lane in 0..count {
            self.check_cancel()?;
            let start = lane * lane_step;
            order.clear();
            order.extend(0..len);
            self.cxr_sort_by(&mut order, |a, b| {
                let (a, b) = (&keys[start + a * step], &keys[start + b * step]);
                if descend { b.cmp(a) } else { a.cmp(b) }
            })?;
            for (k, &from) in order.iter().enumerate() {
                self.cxr_poll(k)?;
                sorted[start + k * step] = data[start + from * step];
                if want_indices {
                    indices[start + k * step] = (from + 1) as f64;
                }
            }
        }
        Ok((
            Value::Complex(self.cxr_build(rows, cols, sorted)?),
            want_indices
                .then(|| Matrix::from_column_vec(rows, cols, indices))
                .transpose()?,
        ))
    }

    /// Stable row order by the given 1-based columns (negative: descending).
    fn cxr_row_order(&self, z: &ComplexMatrix, columns: &[(usize, bool)]) -> Result<Vec<usize>> {
        let rows = z.rows();
        let keys = self.cxr_keys(z.data())?;
        let mut order: Vec<usize> = (0..rows).collect();
        self.cxr_sort_by(&mut order, |a, b| {
            columns
                .iter()
                .map(|&(c, descend)| {
                    let (x, y) = (&keys[a + c * rows], &keys[b + c * rows]);
                    if descend { y.cmp(x) } else { x.cmp(y) }
                })
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        })?;
        self.check_cancel()?;
        Ok(order)
    }

    fn cxr_take_rows(&self, z: &ComplexMatrix, picked: &[usize]) -> Result<ComplexMatrix> {
        let (rows, cols) = (z.rows(), z.cols());
        let mut data = Vec::with_capacity(picked.len() * cols);
        for c in 0..cols {
            self.cxr_poll(c)?;
            data.extend(picked.iter().map(|&r| z.data()[r + c * rows]));
        }
        self.cxr_build(picked.len(), cols, data)
    }

    /// [B,I] = sortrows(A[,column]) in the complex order; B narrows to real
    /// (the result selects rows from A).
    pub(super) fn cxr_sortrows(
        &self,
        args: &[Value],
        want_indices: bool,
    ) -> Result<(Value, Option<Matrix>)> {
        if args.len() > 2 {
            return Err(TOO_MANY.into());
        }
        let z = complex_arg(&args[0]);
        let cols = z.cols();
        let columns: Vec<(usize, bool)> = match args.get(1) {
            None => (0..cols).map(|c| (c, false)).collect(),
            Some(Value::Matrix(m)) if m.rows() <= 1 || m.cols() == 1 => m
                .data()
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    self.cxr_poll(i)?;
                    if v.fract() != 0. || v == 0. || v.abs() > cols as f64 {
                        Err("sortrows: column numbers must be nonzero integers no larger than the number of columns".to_string())
                    } else {
                        Ok((v.abs() as usize - 1, v < 0.))
                    }
                })
                .collect::<Result<_>>()?,
            Some(_) => {
                return Err(
                    "sortrows: complex input supports a numeric column vector only".into(),
                );
            }
        };
        // The real path's preflights: evaluation budget (output plus the key
        // table, and the index column when asked), then the row x key work bound.
        let rows = z.rows();
        let outputs = z
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(if want_indices { rows } else { 0 }))
            .and_then(|n| self.evaluated_elements.checked_add(n));
        if outputs.is_none_or(|n| n > 32 * 1024 * 1024) {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        if rows > 1 {
            let levels = usize::BITS as usize - (rows - 1).leading_zeros() as usize;
            if rows
                .checked_mul(columns.len())
                .and_then(|n| n.checked_mul(levels + 2))
                .is_none_or(|n| n > MAX_SORT_WORK)
            {
                return Err("sortrows exceeds its bounded row/key work budget".into());
            }
        }
        let order = self.cxr_row_order(z, &columns)?;
        let sorted = self.cxr_take_rows(z, &order)?;
        Ok((
            self.narrowed(sorted)?,
            want_indices
                .then(|| {
                    Matrix::from_column_vec(
                        order.len(),
                        1,
                        order.iter().map(|&r| (r + 1) as f64).collect(),
                    )
                })
                .transpose()?,
        ))
    }

    /// [C,ia,ic] = unique(A[,'rows'][,'sorted'|'stable'][,'first'|'last']) in
    /// the complex order. Equal means exactly equal, so NaNs stay distinct.
    /// C narrows to real; ia and ic are columns.
    pub(super) fn cxr_unique(&self, args: &[Value], outputs: [bool; 3]) -> Result<Vec<Value>> {
        let (mut by_rows, mut stable, mut last) = (false, false, false);
        for value in &args[1..] {
            let text = match value {
                Value::Text(_) | Value::QuotedString(_) | Value::Container(Array::Character(_)) => {
                    self.text_value(value.clone())?
                }
                _ => {
                    return Err(
                        "unique: options must be 'rows', 'sorted', 'stable', 'first' or 'last'"
                            .into(),
                    );
                }
            };
            match text.to_ascii_lowercase().as_str() {
                "rows" => by_rows = true,
                "stable" => stable = true,
                "sorted" => stable = false,
                "first" => last = false,
                "last" => last = true,
                "legacy" => {
                    return Err("unique: 'legacy' is not supported for complex input".into());
                }
                other => return Err(format!("unique: unknown option '{other}'")),
            }
        }
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        self.check_cancel()?;
        let items = if by_rows { rows } else { z.len() };
        // Match unique_call's conservative requested-output preflight, before
        // allocating keys or index scratch. Ignored outputs cost zero. A complex
        // element is two allocation units (Value::allocation_units).
        if z.len()
            .checked_mul(2 * usize::from(outputs[0]))
            .and_then(|n| n.checked_add(if outputs[1] { items } else { 0 }))
            .and_then(|n| n.checked_add(if outputs[2] { items } else { 0 }))
            .and_then(|n| self.evaluated_elements.checked_add(n))
            .is_none_or(|n| n > 32 * 1024 * 1024)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let data = z.data();
        // Items are elements, or rows with 'rows'; each group lists its items.
        let (items, order) = if by_rows {
            let all: Vec<(usize, bool)> = (0..cols).map(|c| (c, false)).collect();
            (rows, self.cxr_row_order(z, &all)?)
        } else {
            let keys = self.cxr_keys(data)?;
            let mut order: Vec<usize> = (0..data.len()).collect();
            self.cxr_sort_by(&mut order, |a, b| keys[a].cmp(&keys[b]))?;
            (data.len(), order)
        };
        let equal = |a: usize, b: usize| {
            if by_rows {
                (0..cols).all(|c| data[a + c * rows] == data[b + c * rows])
            } else {
                data[a] == data[b]
            }
        };
        self.check_cancel()?;
        // (first item, last item) of each group, in sorted order.
        let mut groups: Vec<(usize, usize)> = Vec::new();
        let mut group_of = vec![0usize; if outputs[2] { items } else { 0 }];
        for (k, &item) in order.iter().enumerate() {
            self.cxr_poll(k)?;
            if k == 0 || !equal(order[k - 1], item) {
                groups.push((item, item));
            } else {
                let g = groups.last_mut().unwrap();
                g.0 = g.0.min(item);
                g.1 = g.1.max(item);
            }
            if outputs[2] {
                group_of[item] = groups.len() - 1;
            }
        }
        let mut position: Vec<usize> = (0..groups.len()).collect();
        if stable {
            let mut by_first: Vec<usize> = (0..groups.len()).collect();
            self.cxr_sort_by(&mut by_first, |a, b| groups[a].0.cmp(&groups[b].0))?;
            for (p, &g) in by_first.iter().enumerate() {
                position[g] = p;
            }
        }
        let mut picked = vec![0usize; groups.len()];
        for (g, &(first, final_item)) in groups.iter().enumerate() {
            picked[position[g]] = if last { final_item } else { first };
        }
        self.check_cancel()?;
        let empty = || Matrix::filled(0, 0, 0.).map(Value::Matrix);
        let unique = if outputs[0] {
            let unique = if by_rows {
                self.cxr_take_rows(z, &picked)?
            } else {
                let values = picked.iter().map(|&i| data[i]).collect();
                if rows == 1 {
                    self.cxr_build(1, picked.len(), values)?
                } else {
                    self.cxr_build(picked.len(), 1, values)?
                }
            };
            self.narrowed(unique)?
        } else {
            empty()?
        };
        let column = |v: Vec<f64>| Matrix::from_column_vec(v.len(), 1, v).map(Value::Matrix);
        let first = if outputs[1] {
            column(picked.iter().map(|&i| (i + 1) as f64).collect())?
        } else {
            empty()?
        };
        let inverse = if outputs[2] {
            column(group_of.iter().map(|&g| (position[g] + 1) as f64).collect())?
        } else {
            empty()?
        };
        self.check_cancel()?;
        Ok(vec![unique, first, inverse])
    }

    /// round/floor/ceil/fix on each part, plus round(A,n); narrows to real.
    fn cxr_round(&self, name: &str, args: &[Value]) -> Result<Value> {
        let digits = match args.len() {
            1 => None,
            2 | 3 if name == "round" => {
                if let Some(mode) = args.get(2)
                    && !matches!(mode, Value::Text(s) if s.eq_ignore_ascii_case("decimals"))
                {
                    return Err("round: complex input supports round(A), round(A,n) and round(A,n,'decimals') only".into());
                }
                match &args[1] {
                    Value::Matrix(n) if n.len() == 1 && n.data()[0].fract() == 0. => {
                        Some(n.data()[0].clamp(-400., 400.) as i32)
                    }
                    _ => return Err("round: N must be an integer scalar".into()),
                }
            }
            _ => return Err(TOO_MANY.into()),
        };
        let f = |x: f64| match (name, digits) {
            ("round", Some(n)) => round_digits(x, n),
            ("round", None) => x.round(),
            ("floor", _) => x.floor(),
            ("ceil", _) => x.ceil(),
            _ => x.trunc(),
        };
        let z = complex_arg(&args[0]);
        let mut data = Vec::with_capacity(z.len());
        for chunk in z.data().chunks(8192) {
            self.check_cancel()?;
            data.extend(chunk.iter().map(|v| C::new(f(v.re), f(v.im))));
        }
        self.narrowed(self.cxr_build(z.rows(), z.cols(), data)?)
    }

    /// flip(A[,dim]), fliplr and flipud keep complex storage (and signed zeros).
    fn cxr_flip(&self, name: &str, args: &[Value]) -> Result<Value> {
        if args.len() > if name == "flip" { 2 } else { 1 } {
            return Err(TOO_MANY.into());
        }
        let z = complex_arg(&args[0]);
        let (rows, cols) = (z.rows(), z.cols());
        let dim = match name {
            "flipud" => 1,
            "fliplr" => 2,
            _ => match args.get(1) {
                Some(d) => dim_of(d)?,
                None => first_dim(rows, cols),
            },
        };
        if dim > 2 {
            return Ok(Value::Complex(z.clone()));
        }
        let mut data = Vec::with_capacity(z.len());
        for c in 0..cols {
            if c % 1024 == 0 {
                self.check_cancel()?;
            }
            for r in 0..rows {
                let (r, c) = if dim == 1 {
                    (rows - 1 - r, c)
                } else {
                    (r, cols - 1 - c)
                };
                data.push(z.data()[r + c * rows]);
            }
        }
        Ok(Value::Complex(self.cxr_build(rows, cols, data)?))
    }

    /// The reference's message when reshape(A,m,n,...) or reshape(A,[m n ...]) asks
    /// for another element count; other forms keep the shared checks.
    fn cxr_reshape_check(&self, args: &[Value]) -> Result<()> {
        let mut sizes = Vec::new();
        for value in &args[1..] {
            match value {
                Value::Matrix(m) if args.len() == 2 || m.len() == 1 => {
                    sizes.extend_from_slice(m.data())
                }
                _ => return Ok(()),
            }
        }
        if sizes.is_empty() || sizes.iter().any(|&v| !(v >= 0. && v.fract() == 0.)) {
            return Ok(());
        }
        let count = sizes.iter().product::<f64>();
        if count != complex_arg(&args[0]).len() as f64 {
            return Err("To RESHAPE the number of elements must not change.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, sync::atomic::Ordering as AtomicOrdering};

    #[test]
    fn dispersion_lane_scales_and_skips_missing_values() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let lane = |values: &[C], population, omit, kind| {
            dispersion_lane(|k| values[k], values.len(), population, omit, kind, &cancel).unwrap()
        };
        // |z - mean|^2 = 8e600 overflows unscaled; std is still representable.
        let huge = [C::new(1e300, 1e300), C::new(-1e300, -1e300)];
        let std = lane(&huge, false, false, Dispersion::StandardDeviation);
        assert!((std - 2e300).abs() <= 1e-15 * 2e300, "{std:e}");
        assert_eq!(
            lane(&huge, true, false, Dispersion::Variance),
            f64::INFINITY
        );
        let missing = [C::new(0., 1.), C::new(f64::NAN, 0.), C::new(3., 0.)];
        assert!(lane(&missing, false, false, Dispersion::Variance).is_nan());
        let v = lane(&missing, false, true, Dispersion::Variance);
        assert!((v - 5.).abs() <= 4. * f64::EPSILON * 5., "{v}");
        assert_eq!(
            lane(&[C::new(2., 7.)], false, false, Dispersion::Variance),
            0.
        );
        assert!(
            lane(
                &[C::new(f64::INFINITY, 0.), C::new(1., 0.)],
                false,
                false,
                Dispersion::Variance
            )
            .is_nan()
        );
        assert!(lane(&[], false, false, Dispersion::Variance).is_nan());
    }

    #[test]
    fn dispersion_lane_keeps_small_deviations_of_large_values() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let lane = |values: &[C], kind| {
            dispersion_lane(|k| values[k], values.len(), false, false, kind, &cancel).unwrap()
        };
        let close = |got: f64, want: f64| (got - want).abs() <= 4. * f64::EPSILON * want;
        // Scaled by the 1e200 real part, the +-1i deviations square to 1e-401 and vanish.
        let offset = [C::new(1e200, 1.), C::new(1e200, -1.)];
        let var = lane(&offset, Dispersion::Variance);
        assert!(close(var, 2.), "{var:e}");
        let std = lane(&offset, Dispersion::StandardDeviation);
        assert!(close(std, std::f64::consts::SQRT_2), "{std:e}");
        // Same for tiny values: the variance 2e-500 underflows, the std 1.4e-250 does not.
        let small = [C::new(1e-200, 1e-250), C::new(1e-200, -1e-250)];
        assert_eq!(lane(&small, Dispersion::Variance), 0.);
        let std = lane(&small, Dispersion::StandardDeviation);
        assert!(close(std, std::f64::consts::SQRT_2 * 1e-250), "{std:e}");
        // Identical values (the mean of four is exact) have no spread, whatever their size.
        let same = [C::new(1e300, -1e300); 4];
        assert_eq!(lane(&same, Dispersion::Variance), 0.);
    }

    thread_local! {
        static POLLS: Cell<usize> = const { Cell::new(0) };
        static CANCEL_AT: Cell<Option<usize>> = const { Cell::new(None) };
    }

    /// Counts the merge-pass polls of `cxr_sort_by`; cancels at the armed one.
    pub(super) fn cancel_after_merge_progress(engine: &Engine) {
        let seen = POLLS.with(|polls| {
            polls.set(polls.get() + 1);
            polls.get()
        });
        if CANCEL_AT.with(Cell::get).is_some_and(|at| seen >= at) {
            CANCEL_AT.with(|at| at.set(None));
            engine.cancel.store(true, AtomicOrdering::Relaxed);
        }
    }

    fn reversed(engine: &Engine, n: usize) -> Value {
        let data = (0..n).map(|i| C::new((n - i) as f64, 0.)).collect();
        Value::Complex(ComplexMatrix::from_column_vec(n, 1, data, &engine.cancel).unwrap())
    }

    #[test]
    fn complex_order_is_abs_then_angle_with_nan_last() {
        let mut values = [
            C::new(2., 0.),
            C::new(-2., 0.),
            C::new(0., 2.),
            C::new(0., -2.),
            C::new(f64::NAN, 0.),
            C::new(1., 0.),
        ];
        values.sort_by(|a, b| Key::of(*a).cmp(&Key::of(*b)));
        let expected = [(1., 0.), (0., -2.), (2., 0.), (0., 2.), (-2., 0.)];
        for (v, e) in values.iter().zip(expected) {
            assert_eq!((v.re, v.im), e);
        }
        assert!(values[5].re.is_nan());
        // -0 imaginary parts still sort as angle pi, not -pi.
        assert_eq!(
            Key::of(C::new(-2., -0.)).cmp(&Key::of(C::new(0., 2.))),
            Ordering::Greater
        );
    }

    #[test]
    fn median_mean_of_and_round_digits() {
        assert_eq!(mean_of(C::new(-2., 0.), C::new(2., 2.)), C::new(0., 1.));
        assert_eq!(mean_of(C::new(1., 1.), C::new(3., 3.)), C::new(2., 2.));
        assert_eq!(round_digits(1.26, 1), 1.3);
        assert_eq!(round_digits(1250., -2), 1300.);
        assert_eq!(round_digits(-0.4, 0), -0.);
        assert!(round_digits(f64::NAN, 2).is_nan());
    }

    #[test]
    fn merge_sort_keeps_the_stable_order_of_sort_by() {
        let e = Engine::default();
        let keys: Vec<usize> = (0..20000).map(|i| (i * 7919) % 13).collect();
        let mut order: Vec<usize> = (0..keys.len()).collect();
        e.cxr_sort_by(&mut order, |a, b| keys[a].cmp(&keys[b]))
            .unwrap();
        let mut expected: Vec<usize> = (0..keys.len()).collect();
        expected.sort_by_key(|&i| keys[i]);
        assert_eq!(order, expected);
    }

    #[test]
    fn every_complex_ordering_path_cancels_after_merge_progress() {
        type Path = fn(&Engine, Value) -> Result<()>;
        let paths: [(&str, Path); 8] = [
            ("sort", |e, z| e.cxr_sort(&[z], true).map(drop)),
            ("median", |e, z| e.cxr_median(&[z]).map(drop)),
            ("mode", |e, z| e.cxr_mode(&[z], 2).map(drop)),
            ("unique", |e, z| e.cxr_unique(&[z], [true; 3]).map(drop)),
            ("unique stable", |e, z| {
                e.cxr_unique(&[z, Value::Text("stable".into())], [true; 3])
                    .map(drop)
            }),
            ("unique rows", |e, z| {
                e.cxr_unique(&[z, Value::Text("rows".into())], [true; 3])
                    .map(drop)
            }),
            ("sortrows", |e, z| e.cxr_sortrows(&[z], true).map(drop)),
            ("sortrows column", |e, z| {
                let column = Matrix::scalar(-1.);
                e.cxr_sortrows(&[z, Value::Matrix(column)], false).map(drop)
            }),
        ];
        let mut unique_polls = 0;
        for (name, run) in paths {
            let e = Engine::default();
            let z = reversed(&e, 32768);
            POLLS.with(|polls| polls.set(0));
            run(&e, z.clone()).unwrap();
            let total = POLLS.with(Cell::get);
            // 32768 items sort in three merge passes of eight polls each.
            assert!(total >= 24, "{name}: {total} merge polls");
            if name == "unique" {
                unique_polls = total;
            }
            if name == "unique stable" {
                // The second sort (groups by first item) is as long as the first.
                assert_eq!(total, 2 * unique_polls);
            }
            let cancel_at = total * 3 / 4;
            if name == "unique stable" {
                assert!(cancel_at > unique_polls, "must reach the second sort");
            }
            POLLS.with(|polls| polls.set(0));
            CANCEL_AT.with(|at| at.set(Some(cancel_at)));
            let message = run(&e, z).unwrap_err();
            assert!(message.contains("cancelled"), "{name}: {message}");
            assert_eq!(POLLS.with(Cell::get), cancel_at, "{name}");
            assert!(cancel_at > 1, "{name}: must make merge progress");
            assert!(CANCEL_AT.with(|at| at.get().is_none()), "{name}");
        }
    }

    #[test]
    fn complex_unique_honours_output_masks_and_preflights_budget() {
        let mut e = Engine::default();
        let z = ComplexMatrix::from_column_vec(
            1,
            3,
            vec![C::new(2., 1.), C::new(1., 1.), C::new(2., 1.)],
            &e.cancel,
        )
        .unwrap();
        for option in ["sorted", "stable", "rows"] {
            let args = [Value::Complex(z.clone()), Value::Text(option.into())];
            let full = e
                .cel_unique_outputs(&args, [true; 3], &mut Execution::default())
                .unwrap();
            for bits in 0..8 {
                let mask = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
                let result = e
                    .cel_unique_outputs(&args, mask, &mut Execution::default())
                    .unwrap();
                for i in 0..3 {
                    if mask[i] {
                        match (&result[i], &full[i]) {
                            (Value::Complex(a), Value::Complex(b)) => {
                                assert_eq!(a.data(), b.data())
                            }
                            (Value::Matrix(a), Value::Matrix(b)) => assert_eq!(a.data(), b.data()),
                            _ => panic!("unexpected unique output type"),
                        }
                    } else {
                        assert!(
                            matches!(&result[i], Value::Matrix(a) if a.rows() == 0 && a.cols() == 0)
                        );
                    }
                }
            }
        }
        e.evaluated_elements = 32 * 1024 * 1024 - 5;
        let args = [Value::Complex(z)];
        let before = e.evaluated_elements;
        POLLS.with(|polls| polls.set(0));
        assert!(
            e.cel_unique_outputs(&args, [true, true, false], &mut Execution::default())
                .unwrap_err()
                .contains("evaluation budget")
        );
        assert_eq!(POLLS.with(Cell::get), 0);
        assert!(
            e.cel_unique_outputs(&args, [false, false, true], &mut Execution::default())
                .is_ok()
        );
        assert!(
            e.cel_unique_outputs(&args, [false; 3], &mut Execution::default())
                .is_ok()
        );
        assert_eq!(e.evaluated_elements, before);
    }

    #[test]
    fn sortrows_rejects_over_budget_requests_before_sorting() {
        let e = Engine::default();
        let keys = Matrix::from_column_vec(1, 600, vec![1.; 600]).unwrap();
        let args = [reversed(&e, 1 << 20), Value::Matrix(keys)];
        let message = e.cxr_sortrows(&args, false).unwrap_err();
        assert!(message.contains("row/key work budget"), "{message}");
        assert_eq!(POLLS.with(Cell::get), 0);

        let e = Engine {
            evaluated_elements: 32 * 1024 * 1024 - 13,
            ..Engine::default()
        };
        let data = (0..6).map(|i| C::new(i as f64, 0.)).collect();
        let z = ComplexMatrix::from_column_vec(2, 3, data, &e.cancel).unwrap();
        let args = [Value::Complex(z)];
        let message = e.cxr_sortrows(&args, true).unwrap_err();
        assert!(message.contains("evaluation budget"), "{message}");
        assert!(e.cxr_sortrows(&args, false).is_ok());
    }
}
