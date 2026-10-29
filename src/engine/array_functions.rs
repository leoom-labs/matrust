//! prod, dot, cross, kron, circshift, trace, logspace, meshgrid and ndgrid.
//! Real, logical and complex inputs share one generic kernel per function;
//! every loop over user data checks cancellation per column or lane.
use super::{Engine, Value};
use crate::{Complex64, ComplexMatrix, Matrix, Result};
use std::{
    ops::{Add, Mul, Sub},
    sync::atomic::{AtomicBool, Ordering},
};

trait Num: Copy + Add<Output = Self> + Mul<Output = Self> + Sub<Output = Self> {
    const ZERO: Self;
    const ONE: Self;
    fn conj(self) -> Self;
    fn wrap(out: Out<Self>, cancel: &AtomicBool) -> Result<Value>;
}
impl Num for f64 {
    const ZERO: Self = 0.;
    const ONE: Self = 1.;
    fn conj(self) -> Self {
        self
    }
    fn wrap(out: Out<Self>, _: &AtomicBool) -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_vec(
            out.rows, out.cols, out.data,
        )?))
    }
}
impl Num for Complex64 {
    const ZERO: Self = Complex64::new(0., 0.);
    const ONE: Self = Complex64::new(1., 0.);
    fn conj(self) -> Self {
        Complex64::new(self.re, -self.im)
    }
    fn wrap(out: Out<Self>, cancel: &AtomicBool) -> Result<Value> {
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            out.rows, out.cols, out.data, cancel,
        )?))
    }
}

struct Out<T> {
    rows: usize,
    cols: usize,
    data: Vec<T>,
}
#[derive(Clone, Copy)]
struct View<'a, T> {
    rows: usize,
    cols: usize,
    data: &'a [T],
}
impl<'a> From<&'a Matrix> for View<'a, f64> {
    fn from(m: &'a Matrix) -> Self {
        Self {
            rows: m.rows(),
            cols: m.cols(),
            data: m.data(),
        }
    }
}
impl<'a> From<&'a ComplexMatrix> for View<'a, Complex64> {
    fn from(m: &'a ComplexMatrix) -> Self {
        Self {
            rows: m.rows(),
            cols: m.cols(),
            data: m.data(),
        }
    }
}

enum Operand {
    Real(Matrix),
    Complex(ComplexMatrix),
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}

/// First non-singleton dimension of a 2-D array (MATLAB default for reductions).
fn default_dim(rows: usize) -> usize {
    if rows == 1 { 2 } else { 1 }
}

fn reduce_dim<T: Num>(x: View<T>, dim: Option<usize>, cancel: &AtomicBool) -> Result<Out<T>> {
    if dim.is_none() && x.rows == 0 && x.cols == 0 {
        return Ok(Out {
            rows: 1,
            cols: 1,
            data: vec![T::ONE],
        });
    }
    match dim.unwrap_or(default_dim(x.rows)) {
        1 => {
            let mut data = Vec::with_capacity(x.cols);
            for column in x.data.chunks_exact(x.rows.max(1)).take(x.cols) {
                cancelled(cancel)?;
                data.push(column.iter().fold(T::ONE, |p, &v| p * v));
            }
            data.resize(x.cols, T::ONE);
            Ok(Out {
                rows: 1,
                cols: x.cols,
                data,
            })
        }
        2 => {
            let mut data = vec![T::ONE; x.rows];
            for column in x.data.chunks_exact(x.rows.max(1)).take(x.cols) {
                cancelled(cancel)?;
                for (p, &v) in data.iter_mut().zip(column) {
                    *p = *p * v;
                }
            }
            Ok(Out {
                rows: x.rows,
                cols: 1,
                data,
            })
        }
        _ => Ok(Out {
            rows: x.rows,
            cols: x.cols,
            data: x.data.to_vec(),
        }),
    }
}

fn is_vector(rows: usize, cols: usize) -> bool {
    rows == 1 || cols == 1
}

fn dot_kernel<T: Num>(
    a: View<T>,
    b: View<T>,
    dim: Option<usize>,
    cancel: &AtomicBool,
) -> Result<Out<T>> {
    if dim.is_none() && is_vector(a.rows, a.cols) && is_vector(b.rows, b.cols) {
        if a.data.len() != b.data.len() {
            return Err("dot: vectors must have the same length".into());
        }
        let sum = a
            .data
            .iter()
            .zip(b.data)
            .fold(T::ZERO, |s, (&x, &y)| s + x.conj() * y);
        return Ok(Out {
            rows: 1,
            cols: 1,
            data: vec![sum],
        });
    }
    if (a.rows, a.cols) != (b.rows, b.cols) {
        return Err("dot: A and B must be the same size".into());
    }
    let products: Vec<T> = a
        .data
        .iter()
        .zip(b.data)
        .map(|(&x, &y)| x.conj() * y)
        .collect();
    cancelled(cancel)?;
    let p = View {
        rows: a.rows,
        cols: a.cols,
        data: &products,
    };
    let dim = dim.unwrap_or(default_dim(a.rows));
    if dim > 2 {
        return Ok(Out {
            rows: a.rows,
            cols: a.cols,
            data: products,
        });
    }
    // Sum along dim.
    let (rows, cols) = if dim == 1 { (1, a.cols) } else { (a.rows, 1) };
    let mut data = vec![T::ZERO; rows * cols];
    for c in 0..p.cols {
        cancelled(cancel)?;
        for r in 0..p.rows {
            let slot = if dim == 1 { c } else { r };
            data[slot] = data[slot] + p.data[r + c * p.rows];
        }
    }
    Ok(Out { rows, cols, data })
}

fn cross_kernel<T: Num>(
    a: View<T>,
    b: View<T>,
    dim: Option<usize>,
    cancel: &AtomicBool,
) -> Result<Out<T>> {
    let vectors = is_vector(a.rows, a.cols)
        && is_vector(b.rows, b.cols)
        && a.data.len() == 3
        && b.data.len() == 3;
    if vectors && dim.is_none() {
        let (x, y) = (a.data, b.data);
        let data = vec![
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ];
        let (rows, cols) = if a.rows == 1 || b.rows == 1 {
            (1, 3)
        } else {
            (3, 1)
        };
        return Ok(Out { rows, cols, data });
    }
    if (a.rows, a.cols) != (b.rows, b.cols) {
        return Err("cross: A and B must be the same size".into());
    }
    let dim = match dim {
        Some(d) => d,
        None => [a.rows, a.cols]
            .iter()
            .position(|&n| n == 3)
            .map(|p| p + 1)
            .ok_or("cross: A and B must have at least one dimension of length 3")?,
    };
    let length = match dim {
        1 => a.rows,
        2 => a.cols,
        _ => 1,
    };
    if length != 3 {
        return Err("cross: A and B must have length 3 along the working dimension".into());
    }
    let (lanes, stride, step) = if dim == 1 {
        (a.cols, 1, a.rows)
    } else {
        (a.rows, a.rows, 1)
    };
    let mut data = vec![T::ZERO; a.data.len()];
    for lane in 0..lanes {
        cancelled(cancel)?;
        let base = lane * step;
        let at = |s: &[T], t: usize| s[base + t * stride];
        let (x, y) = (
            [at(a.data, 0), at(a.data, 1), at(a.data, 2)],
            [at(b.data, 0), at(b.data, 1), at(b.data, 2)],
        );
        data[base] = x[1] * y[2] - x[2] * y[1];
        data[base + stride] = x[2] * y[0] - x[0] * y[2];
        data[base + 2 * stride] = x[0] * y[1] - x[1] * y[0];
    }
    Ok(Out {
        rows: a.rows,
        cols: a.cols,
        data,
    })
}

fn kron_kernel<T: Num>(a: View<T>, b: View<T>, cancel: &AtomicBool) -> Result<Out<T>> {
    let rows = a.rows * b.rows;
    let cols = a.cols * b.cols;
    let total = Matrix::checked_len(rows, cols)?;
    let mut data = Vec::with_capacity(total);
    for j in 0..a.cols {
        for l in 0..b.cols {
            cancelled(cancel)?;
            for i in 0..a.rows {
                let factor = a.data[i + j * a.rows];
                data.extend(
                    b.data[l * b.rows..(l + 1) * b.rows]
                        .iter()
                        .map(|&v| factor * v),
                );
            }
        }
    }
    Ok(Out { rows, cols, data })
}

fn circshift_kernel<T: Copy>(x: View<T>, shifts: [i64; 2], cancel: &AtomicBool) -> Result<Out<T>> {
    let (rows, cols) = (x.rows, x.cols);
    let mut data = x.data.to_vec();
    if rows * cols == 0 {
        return Ok(Out { rows, cols, data });
    }
    let sr = shifts[0].rem_euclid(rows as i64) as usize;
    let sc = shifts[1].rem_euclid(cols as i64) as usize;
    for c in 0..cols {
        cancelled(cancel)?;
        let target = (c + sc) % cols;
        let column = &mut data[target * rows..(target + 1) * rows];
        column.copy_from_slice(&x.data[c * rows..(c + 1) * rows]);
        column.rotate_right(sr);
    }
    Ok(Out { rows, cols, data })
}

fn trace_kernel<T: Num>(x: View<T>) -> Result<Out<T>> {
    if x.rows != x.cols {
        return Err("Matrix must be square.".into());
    }
    let sum = (0..x.rows).fold(T::ZERO, |s, i| s + x.data[i + i * x.rows]);
    Ok(Out {
        rows: 1,
        cols: 1,
        data: vec![sum],
    })
}

fn dimension(name: &str, value: &Matrix) -> Result<usize> {
    let d = value
        .as_scalar()
        .map_err(|_| format!("{name}: the dimension must be a positive integer scalar"))?;
    if !d.is_finite() || d.fract() != 0. || !(1. ..=1e9).contains(&d) {
        return Err(format!(
            "{name}: the dimension must be a positive integer scalar"
        ));
    }
    Ok(d as usize)
}

macro_rules! unary {
    ($engine:expr, $name:expr, $value:expr, |$x:ident| $body:expr) => {
        match $engine.math_operand($name, $value)? {
            Operand::Real(m) => {
                let $x = View::from(&m);
                f64::wrap($body?, &$engine.cancel)
            }
            Operand::Complex(m) => {
                let $x = View::from(&m);
                Complex64::wrap($body?, &$engine.cancel)
            }
        }
    };
}
macro_rules! binary {
    ($engine:expr, $name:expr, $a:expr, $b:expr, |$x:ident, $y:ident| $body:expr) => {
        match (
            $engine.math_operand($name, $a)?,
            $engine.math_operand($name, $b)?,
        ) {
            (Operand::Real(p), Operand::Real(q)) => {
                let ($x, $y) = (View::from(&p), View::from(&q));
                f64::wrap($body?, &$engine.cancel)
            }
            (p, q) => {
                let p = $engine.to_complex(p)?;
                let q = $engine.to_complex(q)?;
                let ($x, $y) = (View::from(&p), View::from(&q));
                Complex64::wrap($body?, &$engine.cancel)
            }
        }
    };
}

impl Engine {
    fn math_operand(&self, name: &str, value: &Value) -> Result<Operand> {
        match value {
            Value::Complex(z) => Ok(Operand::Complex(z.clone())),
            other => Ok(Operand::Real(self.real_arg(name, other)?)),
        }
    }
    fn to_complex(&self, operand: Operand) -> Result<ComplexMatrix> {
        match operand {
            Operand::Complex(z) => Ok(z),
            Operand::Real(m) => ComplexMatrix::from_real(&m, &self.cancel),
        }
    }
    fn optional_dim(&self, name: &str, value: Option<&Value>) -> Result<Option<usize>> {
        value
            .map(|v| dimension(name, &self.real_arg(name, v)?))
            .transpose()
    }

    pub(super) fn array_function(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        match name {
            "prod" => {
                Self::arity_check(name, args, &[1, 2])?;
                let dim = self.optional_dim(name, args.get(1))?;
                unary!(self, name, &args[0], |x| reduce_dim(x, dim, &self.cancel))
            }
            "dot" | "cross" => {
                Self::arity_check(name, args, &[2, 3])?;
                let dim = self.optional_dim(name, args.get(2))?;
                if name == "dot" {
                    binary!(self, name, &args[0], &args[1], |x, y| dot_kernel(
                        x,
                        y,
                        dim,
                        &self.cancel
                    ))
                } else {
                    binary!(self, name, &args[0], &args[1], |x, y| cross_kernel(
                        x,
                        y,
                        dim,
                        &self.cancel
                    ))
                }
            }
            "kron" => {
                if args.len() < 2 {
                    return Err("Not enough input arguments.".into());
                }
                if args.len() > 2 {
                    return Err("Too many input arguments.".into());
                }
                let (a, b) = (self.shape_of(&args[0])?, self.shape_of(&args[1])?);
                self.preflight_elements(Matrix::checked_len(a.0 * b.0, a.1 * b.1)?)?;
                binary!(self, name, &args[0], &args[1], |x, y| kron_kernel(
                    x,
                    y,
                    &self.cancel
                ))
            }
            "circshift" => self.circshift(args),
            "trace" => {
                Self::arity_check(name, args, &[1])?;
                let value = unary!(self, name, &args[0], |x| trace_kernel(x))?;
                // As in MATLAB, a sum whose imaginary part is zero is real.
                match value {
                    Value::Complex(z) if z.data().iter().all(|v| v.im == 0.) => {
                        Ok(Value::Matrix(z.real(&self.cancel)?))
                    }
                    other => Ok(other),
                }
            }
            "logspace" => self.logspace(args),
            _ => Err(format!("{name} is not an array function")),
        }
    }

    fn shape_of(&self, value: &Value) -> Result<(usize, usize)> {
        match value {
            Value::Complex(z) => Ok((z.rows(), z.cols())),
            Value::Matrix(m) => Ok((m.rows(), m.cols())),
            Value::Logical(m) => Ok((m.rows(), m.cols())),
            _ => Err("Expected a numeric or logical value".into()),
        }
    }

    fn circshift(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("circshift", args, &[2, 3])?;
        let amount = self.real_arg("circshift", &args[1])?;
        if amount
            .data()
            .iter()
            .any(|v| !v.is_finite() || v.fract() != 0.)
        {
            return Err("circshift: shifts must be finite integers".into());
        }
        let (rows, cols) = self.shape_of(&args[0])?;
        // Reduce in f64 before the integer cast so huge shifts (1e20) wrap
        // exactly instead of saturating; an empty dimension needs no shift.
        let lengths = [rows, cols];
        let reduce = |v: f64, dim: usize| -> i64 {
            let n = lengths[dim] as f64;
            if n == 0. { 0 } else { v.rem_euclid(n) as i64 }
        };
        let mut shifts = [0i64; 2];
        if let Some(dim) = self.optional_dim("circshift", args.get(2))? {
            if amount.len() != 1 {
                return Err(
                    "circshift: a dimension cannot be combined with a vector of shifts".into(),
                );
            }
            if dim <= 2 {
                shifts[dim - 1] = reduce(amount.data()[0], dim - 1);
            }
        } else if amount.len() == 1 {
            // Reference behavior: a scalar shift without dim always shifts dimension 1
            shifts[0] = reduce(amount.data()[0], 0);
        } else if amount.is_empty() {
            return Err("circshift: the shift must be a nonempty scalar or vector".into());
        } else if amount.rows() != 1 && amount.cols() != 1 {
            return Err("circshift: the shift must be a scalar or a vector".into());
        } else {
            if amount.data()[2.min(amount.len())..]
                .iter()
                .any(|&v| v != 0.)
            {
                return Err(
                    "circshift: shifts along dimensions above 2 are not supported yet".into(),
                );
            }
            for (dim, (slot, &v)) in shifts.iter_mut().zip(amount.data()).enumerate() {
                *slot = reduce(v, dim);
            }
        }
        let cancel = &self.cancel;
        match &args[0] {
            Value::Logical(l) => {
                let numeric = l.to_numeric(cancel)?;
                let out = circshift_kernel(View::from(&numeric), shifts, cancel)?;
                let m = Matrix::from_column_vec(out.rows, out.cols, out.data)?;
                self.logical_from(&m)
            }
            other => unary!(self, "circshift", other, |x| circshift_kernel(
                x, shifts, cancel
            )),
        }
    }

    fn logspace(&self, args: &[Value]) -> Result<Value> {
        Self::arity_check("logspace", args, &[2, 3])?;
        let scalar = |i: usize| -> Result<f64> {
            self.real_arg("logspace", &args[i])?
                .as_scalar()
                .map_err(|_| "logspace: inputs must be real scalars".to_string())
        };
        let (a, b) = (scalar(0)?, scalar(1)?);
        let n = if args.len() == 3 { scalar(2)? } else { 50. };
        if n.is_nan() {
            return Err("logspace: the point count must not be NaN".into());
        }
        let n = n.floor().clamp(0., 1e9) as usize;
        let b = if b == std::f64::consts::PI {
            b.log10()
        } else {
            b
        };
        Matrix::checked_len(1, n)?;
        self.preflight_elements(n)?;
        let step = if n > 1 { (b - a) / (n - 1) as f64 } else { 0. };
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            if i % 65536 == 0 {
                self.check_cancel()?;
            }
            let exponent = if i + 1 == n { b } else { a + i as f64 * step };
            data.push(10f64.powf(exponent));
        }
        Ok(Value::Matrix(Matrix::from_column_vec(1, n, data)?))
    }

    pub(super) fn grid_call(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        if args.is_empty() || args.len() > 3 {
            return Err(format!("{name} expects 1 or 2 input arguments"));
        }
        if args.len() == 3 || count > 2 {
            return Err(format!("{name}: the 3-D form is not supported yet"));
        }
        if name == "meshgrid" && super::cplx_poly::complex_input(args) {
            return self.cxp_meshgrid(args, count);
        }
        let x = self.real_arg(name, &args[0])?;
        let y = match args.get(1) {
            Some(v) => self.real_arg(name, v)?,
            None => x.clone(),
        };
        let (nx, ny) = (x.len(), y.len());
        let meshgrid = name == "meshgrid";
        if !meshgrid && args.len() == 1 && count <= 1 {
            return Ok(vec![Value::Matrix(Matrix::from_column_slice(
                nx,
                1,
                x.data(),
            )?)]);
        }
        let (rows, cols) = if meshgrid { (ny, nx) } else { (nx, ny) };
        self.preflight_elements(Matrix::checked_len(rows, cols)?.saturating_mul(count.max(1)))?;
        // Column c of `repeat` holds source[c] repeated; every column of `tile` is source.
        let repeat = |source: &Matrix| -> Result<Value> {
            let mut data = Vec::with_capacity(rows * cols);
            for &v in source.data() {
                self.check_cancel()?;
                data.extend(std::iter::repeat_n(v, rows));
            }
            Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
        };
        let tile = |source: &Matrix| -> Result<Value> {
            let mut data = Vec::with_capacity(rows * cols);
            for _ in 0..cols {
                self.check_cancel()?;
                data.extend_from_slice(source.data());
            }
            Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
        };
        let mut outputs = vec![if meshgrid { repeat(&x)? } else { tile(&x)? }];
        if count >= 2 {
            outputs.push(if meshgrid { tile(&y)? } else { repeat(&y)? });
        }
        Ok(outputs)
    }
}
