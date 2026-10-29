//! Explicit complex value admission; unsupported consumers cannot project away
//! imaginary components. Real-only calls retain their existing dispatch paths.
use super::{Engine, Value};
use crate::{Array, Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result};

fn arity(name: &str, args: &[Value], counts: &[usize]) -> Result<()> {
    if counts.contains(&args.len()) {
        Ok(())
    } else {
        Err(format!(
            "{name} expects {} arguments",
            counts
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(" or ")
        ))
    }
}
fn dimension(value: &Value, engine: &Engine) -> Result<usize> {
    let n = value.clone().matrix(&engine.cancel)?.as_scalar()?;
    if !n.is_finite() || n < 0. || n.fract() != 0. || n > crate::matrix::MAX_ELEMENTS as f64 {
        Err("Dimension must be a bounded nonnegative integer".into())
    } else {
        Ok(n as usize)
    }
}
fn real_map(a: &Matrix, engine: &Engine, value: impl Fn(f64) -> f64) -> Result<Matrix> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(a.len())
        .map_err(|_| "Cannot allocate component output")?;
    for chunk in a.data().chunks(4096) {
        engine.check_cancel()?;
        values.extend(chunk.iter().copied().map(&value));
    }
    engine.check_cancel()?;
    Matrix::from_column_vec(a.rows(), a.cols(), values)
}
fn real_filled(rows: usize, cols: usize, engine: &Engine) -> Result<Matrix> {
    let count = Matrix::checked_len(rows, cols)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| "Cannot allocate component output")?;
    for first in (0..count).step_by(4096) {
        engine.check_cancel()?;
        values.resize((first + 4096).min(count), 0.);
    }
    engine.check_cancel()?;
    Matrix::from_column_vec(rows, cols, values)
}
impl Engine {
    pub(super) fn complex_arithmetic(
        &mut self,
        op: &crate::syntax::Token,
        a: Value,
        b: Value,
        out: &mut super::Execution,
    ) -> Result<Value> {
        use crate::complex_matrix::ComplexOp;
        use crate::syntax::Token;
        let divide = match op {
            Token::Slash => Some(false),
            Token::Backslash => Some(true),
            _ => None,
        };
        let op=match op {
            Token::Slash | Token::Backslash => ComplexOp::Divide,
            Token::Plus=>ComplexOp::Add,Token::Minus=>ComplexOp::Subtract,
            Token::DotStar=>ComplexOp::Multiply,Token::DotSlash=>ComplexOp::Divide,Token::Star=>ComplexOp::Matmul,
            _=>return Err("Complex arithmetic currently supports +, -, .*, ./ and *; complex powers and linear solves are not implemented".into()),
        };
        let a = a.numeric_operand(&self.cancel)?.array(&self.cancel)?;
        let b = b.numeric_operand(&self.cancel)?.array(&self.cancel)?;
        if let Some(backslash) = divide
            && (if backslash { &a } else { &b }).len() != 1
        {
            return self.cxl_solve(backslash, &a, &b, out);
        }
        let (rows, cols) = ComplexMatrix::arithmetic_shape(&a, &b, op).map_err(|message| {
            if matches!(op, ComplexOp::Matmul)
                && a.is_numeric_array()
                && b.is_numeric_array()
                && a.len() != 1
                && b.len() != 1
                && a.cols() != b.rows()
            {
                self.raise(message, "MATLAB:innerdim".into())
            } else {
                message
            }
        })?;
        let units = ComplexMatrix::checked_len(rows, cols)? * 2;
        let scratch = ComplexMatrix::arithmetic_scratch_bytes(&a, &b, op).div_ceil(8);
        if self
            .evaluated_elements
            .saturating_add(units)
            .saturating_add(scratch)
            > 32 * 1024 * 1024
        {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        if let Some(backslash) = divide {
            return self.cxl_divide(backslash, &a, &b);
        }
        let result = ComplexMatrix::arithmetic(&a, &b, op, &self.cancel)?;
        // As in MATLAB, an arithmetic result whose imaginary parts are all zero
        // is real: 1+0i is real, while complex(1,0) itself stays complex.
        if !result.is_empty() && result.data().iter().all(|z| z.im == 0.) {
            return Ok(Value::Matrix(result.real(&self.cancel)?));
        }
        Ok(Value::Complex(result))
    }

    pub(super) fn concatenate_complex(&self, rows: Vec<Vec<Value>>) -> Result<Value> {
        self.check_cancel()?;
        let rows = rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|value| {
                        let a = value.array(&self.cancel)?;
                        a.require_numeric_array()?;
                        Ok(a)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let rows: Vec<Vec<Array>> = rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .filter(|a| (a.rows(), a.cols()) != (0, 0))
                    .collect()
            })
            .collect();
        let mut height = 0usize;
        let mut width = None;
        for row in &rows {
            if row.is_empty() {
                continue;
            }
            let h = row[0].rows();
            if row.iter().any(|a| a.rows() != h) {
                return Err("Horizontal concatenation requires matching row counts".into());
            }
            let w = row
                .iter()
                .try_fold(0usize, |n, a| n.checked_add(a.cols()))
                .ok_or("Complex concatenation dimensions overflow")?;
            if width.is_some_and(|old| old != w) {
                return Err("Matrix rows must have matching widths".into());
            }
            width = Some(w);
            height = height
                .checked_add(h)
                .ok_or("Complex concatenation dimensions overflow")?;
        }
        let width = width.unwrap_or(0);
        let count = ComplexMatrix::checked_len(height, width)?;
        if self.evaluated_elements.saturating_add(count * 2) > 32 * 1024 * 1024 {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .map_err(|_| "Cannot allocate complex concatenation")?;
        for first in (0..count).step_by(4096) {
            self.check_cancel()?;
            data.resize((first + 4096).min(count), Complex64::new(0., 0.));
        }
        let mut r = 0;
        for row in rows {
            let h = row.first().map_or(0, Array::rows);
            let mut c = 0;
            for a in row {
                for column in 0..a.cols() {
                    for first in (0..h).step_by(4096) {
                        self.check_cancel()?;
                        let n = (h - first).min(4096);
                        let to = &mut data[(c + column) * height + r + first
                            ..(c + column) * height + r + first + n];
                        let range = column * h + first..column * h + first + n;
                        match &a {
                            Array::Character(_)
                            | Array::Cell(_)
                            | Array::Graphics(_)
                            | Array::Struct(_)
                            | Array::Handle(_)
                            | Array::Typed(_) => {
                                unreachable!("numeric concatenation preflight")
                            }
                            Array::Complex(a) => to.copy_from_slice(&a.data()[range]),
                            Array::Numeric(a) => {
                                for (out, &v) in to.iter_mut().zip(&a.data()[range]) {
                                    *out = Complex64::new(v, 0.);
                                }
                            }
                            Array::Logical(a) => {
                                for (out, &v) in to.iter_mut().zip(&a.data()[range]) {
                                    *out = Complex64::new(f64::from(v), 0.);
                                }
                            }
                        }
                    }
                }
                c += a.cols();
            }
            r += h;
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            height,
            width,
            data,
            &self.cancel,
        )?))
    }
    pub(super) fn complex_call(&self, name: &str, args: &[Value]) -> Result<Option<Value>> {
        self.check_cancel()?;
        if matches!(name, "i" | "j") {
            arity(name, args, &[0])?;
            return Ok(Some(Value::Complex(ComplexMatrix::scalar(
                Complex64::new(0., 1.),
                &self.cancel,
            )?)));
        }
        if name == "complex" {
            arity(name, args, &[1, 2])?;
            // complex(z) is an identity for an already complex argument.
            if args.len() == 1
                && let Value::Complex(a) = &args[0]
            {
                return Ok(Some(Value::Complex(a.clone())));
            }
            let real = args[0].view(&self.cancel)?;
            let imaginary = args.get(1).map(|v| v.view(&self.cancel)).transpose()?;
            return Ok(Some(Value::Complex(ComplexMatrix::from_component_views(
                real.as_ref(),
                imaginary.as_ref().map(|v| v.as_ref()),
                &self.cancel,
            )?)));
        }
        if name == "isreal" {
            arity(name, args, &[1])?;
            args[0].shape(&self.cancel)?;
            // Storage property: complex(x,0) remains non-real, including empty.
            return Ok(Some(Value::Logical(LogicalMatrix::scalar(!matches!(
                args[0],
                Value::Complex(_)
            )))));
        }
        if name == "ctranspose" {
            arity(name, args, &[1])?;
            return Ok(Some(match &args[0] {
                Value::Complex(a) => Value::Complex(a.adjoint(&self.cancel)?),
                Value::Matrix(a) => Value::Matrix(a.transpose()),
                Value::Logical(a) => Value::Logical(a.transpose(&self.cancel)?),
                _ => return Err("ctranspose requires an array".into()),
            }));
        }
        if matches!(name, "real" | "imag" | "conj" | "angle") {
            arity(name, args, &[1])?;
            return Ok(Some(match &args[0] {
                Value::Complex(a) => match name {
                    "real" => Value::Matrix(a.real(&self.cancel)?),
                    "imag" => Value::Matrix(a.imag(&self.cancel)?),
                    "conj" => Value::Complex(a.conj(&self.cancel)?),
                    _ => Value::Matrix(a.angle(&self.cancel)?),
                },
                Value::Matrix(a) => match name {
                    "imag" => Value::Matrix(real_filled(a.rows(), a.cols(), self)?),
                    "angle" => Value::Matrix(real_map(a, self, |x| 0.0_f64.atan2(x))?),
                    _ => Value::Matrix(a.clone()),
                },
                Value::Logical(a) => match name {
                    "real" | "conj" => Value::Logical(a.clone()),
                    "imag" => Value::Logical(LogicalMatrix::filled(
                        a.rows(),
                        a.cols(),
                        false,
                        &self.cancel,
                    )?),
                    _ => Value::Matrix(real_filled(a.rows(), a.cols(), self)?),
                },
                _ => return Err(format!("{name} requires an array")),
            }));
        }
        let Some(Value::Complex(a)) = args.first() else {
            return Ok(None);
        };
        let value = match name {
            "abs" => {
                arity(name, args, &[1])?;
                Value::Matrix(a.magnitude(&self.cancel)?)
            }
            "double" => {
                arity(name, args, &[1])?;
                Value::Complex(a.clone())
            }
            "transpose" => {
                arity(name, args, &[1])?;
                Value::Complex(a.transpose(&self.cancel)?)
            }
            "size" | "numel" | "length" => {
                arity(name, args, if name == "size" { &[1, 2] } else { &[1] })?;
                Value::Matrix(if name == "size" && args.len() == 1 {
                    Matrix::from_column_slice(1, 2, &[a.rows() as f64, a.cols() as f64])?
                } else if name == "size" {
                    Matrix::scalar(match dimension(&args[1], self)? {
                        0 => return Err("Dimension index starts at 1".into()),
                        1 => a.rows() as f64,
                        2 => a.cols() as f64,
                        _ => 1.,
                    })
                } else {
                    Matrix::scalar(if name == "numel" || a.is_empty() {
                        a.len()
                    } else {
                        a.rows().max(a.cols())
                    } as f64)
                })
            }
            _ => return Ok(None),
        };
        self.check_cancel()?;
        Ok(Some(value))
    }
}
