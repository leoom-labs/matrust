//! Everyday elementwise functions (mod, rem, fix, sign, hypot, atan2) and the
//! dispatcher for the everyday math modules.
use super::{Engine, Value};
use crate::{Complex64, ComplexMatrix, LogicalMatrix, Matrix, Result};

/// Names owned by the everyday math modules; used for multi-output dispatch.
pub(super) fn has_multiple_outputs(name: &str) -> bool {
    matches!(
        name,
        "meshgrid" | "ndgrid" | "eig" | "svd" | "polyfit" | "xcorr" | "xcov"
    )
}

pub(super) fn binary_function(name: &str) -> Option<fn(f64, f64) -> f64> {
    Some(match name {
        "mod" => mod_value,
        "rem" => rem_value,
        "hypot" => f64::hypot,
        "atan2" => f64::atan2,
        _ => return None,
    })
}

pub(super) fn real_elementwise(
    engine: &Engine,
    a: &Matrix,
    b: &Matrix,
    f: impl Fn(f64, f64) -> f64 + Sync + Send,
) -> Result<Matrix> {
    let (rows, cols) = crate::matrix::expansion_shape((a.rows(), a.cols()), (b.rows(), b.cols()))?;
    engine.preflight_elements(Matrix::checked_len(rows, cols)?)?;
    a.zip_checked(b, f, || engine.check_cancel())
}

/// MATLAB's round-off compensation: with a non-integer divisor, a quotient
/// within relative eps of an integer makes the result exactly 0 (so
/// mod(0.3,0.1) is 0 although 0.3/0.1 = 2.9999999999999996). Integer divisors
/// keep the exact remainder (mod(1+eps,1) stays eps). Finite x, y != 0 only.
fn within_roundoff(x: f64, y: f64) -> bool {
    if y.fract() == 0. {
        return false;
    }
    let q = x / y;
    q != 0. && (q - q.round()).abs() <= f64::EPSILON * q.abs()
}

/// x - floor(x./y).*y; the remainder itself comes from the exact `%` (fmod),
/// which equals that formula without its intermediate rounding.
pub(super) fn mod_value(x: f64, y: f64) -> f64 {
    if y == 0. {
        return x;
    }
    if !x.is_finite() || !y.is_finite() {
        return f64::NAN;
    }
    if x == 0. {
        return x;
    }
    if within_roundoff(x, y) {
        return 0.;
    }
    let r = x % y;
    if r == 0. {
        // Reference behavior: exact multiples inherit the divisor's zero sign, while
        // zero inputs and round-off compensation above retain their rules.
        0_f64.copysign(y)
    } else if (r < 0.) != (y < 0.) {
        r + y
    } else {
        r
    }
}

/// x - fix(x./y).*y, with the same exact remainder and round-off rule as mod.
fn rem_value(x: f64, y: f64) -> f64 {
    if y == 0. || !x.is_finite() || !y.is_finite() {
        return f64::NAN;
    }
    if within_roundoff(x, y) { 0. } else { x % y }
}

fn sign_value(x: f64) -> f64 {
    if x.is_nan() {
        f64::NAN
    } else if x > 0. {
        1.
    } else if x < 0. {
        -1.
    } else {
        0.
    }
}

impl Engine {
    /// Real-valued numeric view of an argument; logical converts, complex and
    /// character codes convert through Value; unsupported inputs retain named errors.
    pub(super) fn real_arg(&self, name: &str, value: &Value) -> Result<Matrix> {
        match value {
            Value::Matrix(m) => Ok(m.clone()),
            Value::Logical(l) => l.to_numeric(&self.cancel),
            value if value.is_character() => value.clone().matrix(&self.cancel),
            Value::Complex(_) => Err(format!("{name}: complex input is not supported yet")),
            _ => Err(format!("{name}: input must be numeric or logical")),
        }
    }

    pub(super) fn arity_check(name: &str, args: &[Value], allowed: &[usize]) -> Result<()> {
        if allowed.contains(&args.len()) {
            Ok(())
        } else {
            Err(format!(
                "{name} expects {} arguments",
                allowed
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(" or ")
            ))
        }
    }

    /// Reject element counts (8-byte units) that cannot fit in the statement
    /// budget or in the memory budget before any large allocation.
    pub(super) fn preflight_elements(&self, elements: usize) -> Result<()> {
        // An overflowing count exceeds any finite budget.
        let projected = self.evaluated_elements.saturating_add(elements);
        if let Some(units) = self.limits.evaluation_units
            && projected > units
        {
            return Err(super::limits::evaluation_error(units));
        }
        // One allocation of `elements` units must fit the memory budget on its own,
        // whatever the evaluation budget.
        if elements > super::limits::memory_units(&self.limits) {
            return Err(super::limits::memory_error("Statement", &self.limits));
        }
        Ok(())
    }

    /// Single-output dispatch of the everyday math functions. Returns None for
    /// names owned elsewhere so the caller continues its dispatch chain.
    pub(super) fn everyday_math_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut super::Execution,
    ) -> Result<Option<Value>> {
        let value = match name {
            "mod" | "rem" | "hypot" | "atan2" => {
                self.check_cancel()?;
                Self::arity_check(name, args, &[2])?;
                let f = binary_function(name).expect("a binary elementary function");
                if args.iter().any(Value::is_nd) {
                    return super::nd::everyday_binary(self, name, &args[0], &args[1], f).map(Some);
                }
                let a = self.real_arg(name, &args[0])?;
                let b = self.real_arg(name, &args[1])?;
                Value::Matrix(real_elementwise(self, &a, &b, f)?)
            }
            "fix" | "sign" => {
                self.check_cancel()?;
                Self::arity_check(name, args, &[1])?;
                self.fix_or_sign(name, &args[0])?
            }
            "prod" | "dot" | "cross" | "kron" | "circshift" | "trace" | "logspace" => {
                self.check_cancel()?;
                self.array_function(name, args)?
            }
            "rand" | "randn" | "randi" | "rng" => self.random_call(name, args)?,
            "profile" | "profsave" => super::profiler::call(self, name, args, out)?,
            "tic" | "toc" => self.stopwatch_call(name, args, out, true)?,
            "rank" => self.rank_call(args)?,
            "meshgrid" | "ndgrid" | "eig" | "svd" | "polyfit" | "xcorr" | "xcov" => {
                return Ok(Some(self.everyday_outputs_call(name, args, 1)?.remove(0)));
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// Multi-output dispatch; `count` is the number of requested outputs.
    pub(super) fn everyday_outputs_call(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        match name {
            "meshgrid" | "ndgrid" => self.grid_call(name, args, count),
            "eig" => self.eig_call(args, count),
            "svd" => self.svd_call(args, count),
            "polyfit" => self.polyfit_call(args, count),
            "xcorr" => self.xcorr_call(args, count),
            "xcov" => self.xcov_call(args, count),
            _ => Err(format!("{name} does not support {count} outputs")),
        }
    }

    fn fix_or_sign(&self, name: &str, value: &Value) -> Result<Value> {
        match value {
            Value::Complex(z) => {
                let data: Vec<Complex64> = z
                    .data()
                    .iter()
                    .map(|v| {
                        if name == "fix" {
                            Complex64::new(v.re.trunc(), v.im.trunc())
                        } else {
                            let r = v.re.hypot(v.im);
                            if r == 0. {
                                Complex64::new(0., 0.)
                            } else if r.is_nan() {
                                Complex64::new(f64::NAN, f64::NAN)
                            } else {
                                Complex64::new(v.re / r, v.im / r)
                            }
                        }
                    })
                    .collect();
                Ok(Value::Complex(ComplexMatrix::from_column_vec(
                    z.rows(),
                    z.cols(),
                    data,
                    &self.cancel,
                )?))
            }
            other => {
                let m = self.real_arg(name, other)?;
                Ok(Value::Matrix(m.map(if name == "fix" {
                    f64::trunc
                } else {
                    sign_value
                })))
            }
        }
    }

    /// A logical array as logical output (used by shape-preserving functions).
    pub(super) fn logical_from(&self, m: &Matrix) -> Result<Value> {
        Ok(Value::Logical(LogicalMatrix::from_numeric(
            m,
            &self.cancel,
        )?))
    }
}
