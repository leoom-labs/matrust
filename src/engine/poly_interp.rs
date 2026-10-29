//! Polynomials, convolution, interpolation, scalar optimisation and a few
//! array utilities (reference semantics). Private methods use the `polyopt_` prefix.
mod array_ops;
mod interp;
mod optim;
mod poly;

use super::{Engine, Execution, Value};
use crate::{Complex64, ComplexMatrix, Matrix, Result};

const NAMES: &[&str] = &[
    "conv",
    "deconv",
    "roots",
    "poly",
    "polyder",
    "polyint",
    "conv2",
    "interp1",
    "interp2",
    "spline",
    "pchip",
    "fzero",
    "fminsearch",
    "fminbnd",
    "fft2",
    "ifft2",
    "fftshift",
    "ifftshift",
    "mode",
    "histc",
    "histcounts",
];
const BUDGET: usize = 32 * 1024 * 1024;

pub(super) fn real_matrix(value: &Value, who: &str) -> Result<Matrix> {
    match value {
        Value::Matrix(m) => Ok(m.clone()),
        Value::Logical(l) => Matrix::from_column_vec(
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| f64::from(b)).collect(),
        ),
        Value::Complex(_) => Err(format!("{who}: complex input is not supported")),
        _ => Err(format!("{who}: inputs must be numeric or logical arrays")),
    }
}
pub(super) fn is_vector(m: &Matrix) -> bool {
    m.rows() == 1 || m.cols() == 1
}
pub(super) fn matrix_value(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}
pub(super) fn complex_value(
    engine: &Engine,
    rows: usize,
    cols: usize,
    data: Vec<Complex64>,
) -> Result<Value> {
    Ok(Value::Complex(ComplexMatrix::from_column_vec(
        rows,
        cols,
        data,
        &engine.cancel,
    )?))
}
/// Complex output that collapses to a real double when every imaginary part is zero.
pub(super) fn narrowed_value(
    engine: &Engine,
    rows: usize,
    cols: usize,
    data: Vec<Complex64>,
) -> Result<Value> {
    if data.iter().all(|z| z.im == 0.) {
        matrix_value(rows, cols, data.iter().map(|z| z.re).collect())
    } else {
        complex_value(engine, rows, cols, data)
    }
}
/// Spacing of doubles at |x| (MATLAB eps(x)).
pub(super) fn eps_of(x: f64) -> f64 {
    let a = x.abs();
    if !a.is_finite() {
        return f64::NAN;
    }
    if a < f64::MIN_POSITIVE {
        return f64::from_bits(1);
    }
    f64::from_bits(a.to_bits() + 1) - a
}
/// A scalar option, as a plain f64 (logical true/false allowed).
pub(super) fn scalar_of(value: &Value, who: &str, what: &str) -> Result<f64> {
    let m = real_matrix(value, who)?;
    if m.len() != 1 {
        return Err(format!("{who}: {what} must be a scalar"));
    }
    Ok(m.data()[0])
}
pub(super) fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

fn max_outputs(name: &str) -> usize {
    match name {
        "deconv" | "histc" | "histcounts" | "mode" | "polyder" => 2,
        "fzero" | "fminsearch" | "fminbnd" => 3,
        _ => 1,
    }
}

impl Engine {
    /// Character rows become text; double-quoted strings stay unsupported.
    fn polyopt_text_arguments(&self, args: &[Value]) -> Result<Vec<Value>> {
        if args.iter().any(|v| matches!(v, Value::QuotedString(_))) {
            return Err("String array storage is not implemented; use char(...) explicitly".into());
        }
        self.text_arguments(args.to_vec())
    }
    pub(super) fn polyopt_admit(&self, elements: usize) -> Result<()> {
        if self
            .evaluated_elements
            .checked_add(elements)
            .is_none_or(|n| n > BUDGET)
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }
    /// Single-output entry for `fn call`; None when `name` is not in this family.
    pub(super) fn polyopt_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        let args = self.polyopt_text_arguments(args)?;
        Ok(Some(self.polyopt_run(name, &args, 1, out)?.remove(0)))
    }
    /// Multiple-output entry for `eval_outputs`.
    pub(super) fn polyopt_outputs(
        &mut self,
        name: &str,
        args: &[crate::syntax::Expr],
        count: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        if count > max_outputs(name) {
            return Err(format!("{name} does not support {count} outputs"));
        }
        let values = self.function_arguments(args, None, out)?;
        let values = self.polyopt_text_arguments(&values)?;
        if super::cplx_poly::routes(name, &values) {
            let outputs = self.cxp_call(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(Some(outputs));
        }
        let outputs = self.polyopt_run(name, &values, count, out)?;
        self.charge_function_outputs(&outputs)?;
        self.check_cancel()?;
        Ok(Some(outputs))
    }
    fn polyopt_run(
        &mut self,
        name: &str,
        args: &[Value],
        nout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if nout > max_outputs(name) {
            return Err("Too many output arguments".into());
        }
        let one = |v: Result<Value>| v.map(|v| vec![v]);
        match name {
            "conv" => one(self.polyopt_conv(args)),
            "deconv" => self.polyopt_deconv(args, nout),
            "roots" => one(self.polyopt_roots(args)),
            "poly" => one(self.polyopt_poly(args)),
            "polyder" => self.polyopt_polyder(args, nout),
            "polyint" => one(self.polyopt_polyint(args)),
            "conv2" => one(self.polyopt_conv2(args)),
            "interp1" => one(self.polyopt_interp1(args)),
            "interp2" => one(self.polyopt_interp2(args)),
            "spline" | "pchip" => one(self.polyopt_spline(name, args)),
            "fzero" | "fminsearch" | "fminbnd" => self.polyopt_optimize(name, args, nout, out),
            "fft2" | "ifft2" => one(self.polyopt_fft2(name, args)),
            "fftshift" | "ifftshift" => one(self.polyopt_shift(name, args)),
            "mode" => self.polyopt_mode(args, nout),
            "histc" => self.polyopt_histc(args, nout),
            _ => self.polyopt_histcounts(args, nout),
        }
    }
}
