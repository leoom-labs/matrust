//! MATLAB reduction forms: sum/mean(A[,dim]), [M,I] = min/max(A[,[],dim]),
//! min/max(A,B), any/all(A[,dim]) and [r,c,v] = find(A[,n[,'first'|'last']]).
//! Like unique and ismember, `mask[i]` says whether output i is requested.
use super::{Engine, Value};
use crate::{
    Array, ComplexMatrix, LogicalMatrix, Matrix, Result,
    logical::ArrayRef,
    matrix::{
        Extreme, MAX_ELEMENTS, find_nonzero, find_nonzero_complex, find_shape, truth_along,
        truth_along_complex,
    },
};

pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "sum" | "mean" | "min" | "max" | "any" | "all" | "find"
    )
}

/// Character arrays reduce as their stored UTF-16 code values and shape and give double results.
enum Input {
    Real(Matrix),
    Logical(LogicalMatrix),
    Complex(ComplexMatrix),
    Char(Matrix),
}

fn index_value(index: Option<Matrix>) -> Result<Value> {
    Ok(Value::Matrix(match index {
        Some(index) => index,
        None => Matrix::filled(0, 0, 0.)?,
    }))
}

fn extreme(name: &str) -> Extreme {
    if name == "min" {
        Extreme::Min
    } else {
        Extreme::Max
    }
}

impl Engine {
    fn reduction_input(&self, name: &str, value: &Value) -> Result<Input> {
        Ok(match value {
            Value::Matrix(m) => Input::Real(m.clone()),
            Value::Logical(m) => Input::Logical(m.clone()),
            Value::Complex(m) => Input::Complex(m.clone()),
            value if value.is_character() => Input::Char(value.clone().matrix(&self.cancel)?),
            // Double-quoted literals keep their earlier code reductions.
            Value::QuotedString(text) => {
                Input::Char(Value::Text(text.clone()).matrix(&self.cancel)?)
            }
            _ => {
                return Err(format!(
                    "{name} requires numeric, logical or character input"
                ));
            }
        })
    }
    /// Real double data: logical and character inputs convert to double.
    fn real_input(&self, name: &str, input: Input) -> Result<Matrix> {
        match input {
            Input::Real(m) | Input::Char(m) => Ok(m),
            Input::Logical(m) => m.to_numeric(&self.cancel),
            Input::Complex(_) => Err(format!(
                "{name} of complex values is not implemented; use real and imag"
            )),
        }
    }
    pub(super) fn reduction_dimension(&self, name: &str, value: &Value) -> Result<usize> {
        let dim = match value {
            Value::Matrix(m) if m.len() == 1 => m.data()[0],
            _ => {
                return Err(format!(
                    "{name} dimension must be a positive integer scalar; 'all', type and NaN flags are not implemented"
                ));
            }
        };
        if !(dim >= 1. && dim.fract() == 0. && dim <= MAX_ELEMENTS as f64) {
            return Err(format!(
                "{name} dimension must be a positive integer <= {MAX_ELEMENTS}"
            ));
        }
        Ok(dim as usize)
    }
    /// `outputs` is the number of assignment targets (1 for an expression).
    pub(super) fn reduction_call(
        &self,
        name: &str,
        args: &[Value],
        outputs: usize,
        mask: [bool; 3],
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let values = match name {
            "sum" | "mean" | "any" | "all" => {
                let (input, dim) = match args {
                    [a] => (a, None),
                    [a, d] => (a, Some(self.reduction_dimension(name, d)?)),
                    _ => {
                        return Err(format!(
                            "Use {name}(A) or {name}(A,dim); 'all', type and NaN flags are not implemented"
                        ));
                    }
                };
                let input = self.reduction_input(name, input)?;
                vec![if matches!(name, "sum" | "mean") {
                    let a = self.real_input(name, input)?;
                    Value::Matrix(a.sum_along(dim, name == "mean", &self.cancel)?)
                } else {
                    let all = name == "all";
                    Value::Logical(match input {
                        Input::Real(m) | Input::Char(m) => {
                            truth_along(ArrayRef::Numeric(&m), all, dim, &self.cancel)?
                        }
                        Input::Logical(m) => {
                            truth_along(ArrayRef::Logical(&m), all, dim, &self.cancel)?
                        }
                        Input::Complex(m) => truth_along_complex(&m, all, dim, &self.cancel)?,
                    })
                }]
            }
            "min" | "max" => self.extreme_call(name, args, outputs, mask)?,
            _ => self.find_call(args, outputs, mask)?,
        };
        self.check_cancel()?;
        Ok(values)
    }
    fn extreme_call(
        &self,
        name: &str,
        args: &[Value],
        outputs: usize,
        mask: [bool; 3],
    ) -> Result<Vec<Value>> {
        let kind = extreme(name);
        let dim = match args {
            [_] => None,
            [a, b] => {
                if outputs > 1 {
                    return Err(format!(
                        "{name} with two matrices to compare and two output arguments is not supported"
                    ));
                }
                let a = self.reduction_input(name, a)?;
                let b = self.reduction_input(name, b)?;
                return Ok(vec![self.extreme_pair(name, kind, a, b)?]);
            }
            [_, b, d] => {
                let empty = match b {
                    Value::Matrix(m) => m.is_empty(),
                    Value::Logical(m) => m.is_empty(),
                    _ => false,
                };
                if !empty {
                    return Err(format!(
                        "{name} with two matrices to compare and a working dimension is not supported"
                    ));
                }
                Some(self.reduction_dimension(name, d)?)
            }
            _ => {
                return Err(format!(
                    "Use {name}(A), {name}(A,[],dim) or {name}(A,B); 'all' and NaN flags are not implemented"
                ));
            }
        };
        Ok(match self.reduction_input(name, &args[0])? {
            Input::Complex(a) => {
                let (values, index) = a.extreme_along(kind, dim, mask[1], &self.cancel)?;
                vec![self.narrowed(values)?, index_value(index)?]
            }
            input => {
                let logical = matches!(input, Input::Logical(_));
                let a = self.real_input(name, input)?;
                let (values, index) = a.extreme_along(kind, dim, mask[1], &self.cancel)?;
                vec![self.as_class(values, logical)?, index_value(index)?]
            }
        })
    }
    /// The reference gives a real result when every imaginary part of a complex one is zero.
    pub(super) fn narrowed(&self, m: ComplexMatrix) -> Result<Value> {
        for chunk in m.data().chunks(8192) {
            self.check_cancel()?;
            if chunk.iter().any(|z| z.im != 0.) {
                return Ok(Value::Complex(m));
            }
        }
        Ok(Value::Matrix(m.real(&self.cancel)?))
    }
    /// min/max of logical data is logical; every other input class gives double.
    fn as_class(&self, m: Matrix, logical: bool) -> Result<Value> {
        Ok(if logical {
            Value::Logical(LogicalMatrix::from_numeric(&m, &self.cancel)?)
        } else {
            Value::Matrix(m)
        })
    }
    fn extreme_pair(&self, name: &str, kind: Extreme, a: Input, b: Input) -> Result<Value> {
        if matches!(a, Input::Complex(_)) || matches!(b, Input::Complex(_)) {
            let complex = |input: Input| match input {
                Input::Complex(m) => Ok(m),
                real => ComplexMatrix::from_real(&self.real_input(name, real)?, &self.cancel),
            };
            let (a, b) = (complex(a)?, complex(b)?);
            if a.len() != 1 && b.len() != 1 && (a.rows(), a.cols()) != (b.rows(), b.cols()) {
                return Err("Matrix dimensions must agree.".into());
            }
            return self.narrowed(a.extreme_pair(&b, kind, &self.cancel)?);
        }
        let logical = matches!((&a, &b), (Input::Logical(_), Input::Logical(_)));
        let (a, b) = (self.real_input(name, a)?, self.real_input(name, b)?);
        // Ties (0 and -0) keep the first operand; NaN loses unless both are NaN.
        let pair = match kind {
            Extreme::Min => a.zip(&b, |x, y| if y < x || x.is_nan() { y } else { x }),
            Extreme::Max => a.zip(&b, |x, y| if y > x || x.is_nan() { y } else { x }),
        }?;
        self.as_class(pair, logical)
    }
    fn find_call(&self, args: &[Value], outputs: usize, mask: [bool; 3]) -> Result<Vec<Value>> {
        let (limit, last) = match args {
            [_] => (None, false),
            [_, n] | [_, n, _] => {
                let n = match n {
                    Value::Matrix(m) if m.len() == 1 => m.data()[0],
                    _ => f64::NAN,
                };
                if !(n >= 1. && n.fract() == 0. && n <= u32::MAX as f64) {
                    return Err("find count n must be a positive integer scalar".into());
                }
                let direction = match args.get(2) {
                    None => None,
                    Some(
                        d @ (Value::Text(_)
                        | Value::QuotedString(_)
                        | Value::Container(Array::Character(_))),
                    ) => Some(self.text_value(d.clone())?),
                    Some(_) => return Err("find direction must be 'first' or 'last'".into()),
                };
                let last = match direction.as_deref() {
                    None => false,
                    Some(d) if d.eq_ignore_ascii_case("first") => false,
                    Some(d) if d.eq_ignore_ascii_case("last") => true,
                    Some(_) => return Err("find direction must be 'first' or 'last'".into()),
                };
                (Some(n as usize), last)
            }
            _ => return Err("Use find(X), find(X,n) or find(X,n,'first'|'last')".into()),
        };
        let input = self.reduction_input("find", &args[0])?;
        let (rows, cols, found) = match &input {
            Input::Real(m) | Input::Char(m) => (
                m.rows(),
                m.cols(),
                find_nonzero(ArrayRef::Numeric(m), limit, last, &self.cancel)?,
            ),
            Input::Logical(m) => (
                m.rows(),
                m.cols(),
                find_nonzero(ArrayRef::Logical(m), limit, last, &self.cancel)?,
            ),
            Input::Complex(m) => (
                m.rows(),
                m.cols(),
                find_nonzero_complex(m, limit, last, &self.cancel)?,
            ),
        };
        let (out_rows, out_cols) = find_shape(rows, cols, found.len());
        let numbers = |f: &dyn Fn(usize) -> f64| -> Result<Value> {
            self.check_cancel()?;
            let values = found.iter().map(|&i| f(i)).collect();
            Ok(Value::Matrix(Matrix::from_column_vec(
                out_rows, out_cols, values,
            )?))
        };
        if outputs <= 1 {
            return Ok(vec![numbers(&|i| (i + 1) as f64)?]);
        }
        let skipped = || index_value(None);
        let stride = rows.max(1);
        let mut values = vec![
            if mask[0] {
                numbers(&|i| (i % stride + 1) as f64)?
            } else {
                skipped()?
            },
            if mask[1] {
                numbers(&|i| (i / stride + 1) as f64)?
            } else {
                skipped()?
            },
        ];
        if outputs > 2 {
            values.push(match &input {
                _ if !mask[2] => skipped()?,
                Input::Real(m) | Input::Char(m) => numbers(&|i| m.data()[i])?,
                Input::Logical(_) => Value::Logical(LogicalMatrix::filled(
                    out_rows,
                    out_cols,
                    true,
                    &self.cancel,
                )?),
                Input::Complex(m) => {
                    let picked = found.iter().map(|&i| m.data()[i]).collect();
                    self.narrowed(ComplexMatrix::from_column_vec(
                        out_rows,
                        out_cols,
                        picked,
                        &self.cancel,
                    )?)?
                }
            });
        }
        Ok(values)
    }
}
