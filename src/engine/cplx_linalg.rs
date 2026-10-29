//! Complex division and the complex branches of the linear-algebra functions.
//! Every private method is prefixed `cxl_` (all `impl Engine` blocks share one
//! method namespace).
use super::{Engine, Execution, Value};
use crate::{Array, Complex64, ComplexMatrix, Matrix, Result};
use nalgebra::DMatrix;

type CMat = DMatrix<Complex64>;

const BUDGET: usize = 32 * 1024 * 1024;
/// Dense factorization work bound (m*n*min(m,n)), as linear_algebra2.
const MAX_WORK: f64 = 4.3e9;
const DIMENSIONS: &str = "Matrix dimensions must agree.";
const SQUARE: &str = "Matrix must be square.";
const ZERO: Complex64 = Complex64::new(0., 0.);
const ONE: Complex64 = Complex64::new(1., 0.);
/// Substitution steps between cancellation checks.
const CANCEL_STEPS: usize = 256;

fn cxl_finite(z: &Complex64) -> bool {
    z.re.is_finite() && z.im.is_finite()
}

/// LAPACK zlange '1': the largest column sum of moduli (a NaN column wins).
fn cxl_norm1(a: &CMat) -> f64 {
    a.column_iter()
        .map(|c| c.iter().map(|z| z.norm()).sum::<f64>())
        .fold(0., |a, b| if b.is_nan() || b > a { b } else { a })
}

/// The first index of the largest modulus (izmax1).
fn cxl_first_max(x: &CMat) -> usize {
    let mut best = (0, f64::NEG_INFINITY);
    for (i, z) in x.iter().enumerate() {
        if z.norm() > best.1 {
            best = (i, z.norm());
        }
    }
    best.0
}

fn cxl_dense(a: &Array) -> CMat {
    CMat::from_fn(a.rows(), a.cols(), |i, j| cxl_element(a, i + j * a.rows()))
}

pub(super) fn cxl_element(a: &Array, i: usize) -> Complex64 {
    match a {
        Array::Complex(z) => z.data()[i],
        Array::Numeric(m) => Complex64::new(m.data()[i], 0.),
        Array::Logical(m) => Complex64::new(f64::from(m.data()[i]), 0.),
        _ => unreachable!("numeric operand preflight"),
    }
}

/// Smith's algorithm; the branch for |d| <= |c| keeps exact cases at +0.
fn cxl_smith(a: f64, b: f64, c: f64, d: f64) -> Complex64 {
    if d.abs() <= c.abs() {
        let r = d / c;
        let den = c + d * r;
        Complex64::new((a + b * r) / den, (b - a * r) / den)
    } else {
        let r = c / d;
        let den = d + c * r;
        Complex64::new((b + a * r) / den, (-a + b * r) / den)
    }
}

/// num / den for one scalar divisor: per component for zero, real and purely
/// imaginary divisors, Smith's algorithm otherwise, on power-of-two rescaled
/// operands when a magnitude is outside the safe range (zladiv's purpose).
fn cxl_scalar_divide(num: Complex64, den: Complex64) -> Complex64 {
    let (a, b, c, d) = (num.re, num.im, den.re, den.im);
    if d == 0. {
        return Complex64::new(a / c, b / c);
    }
    if c == 0. {
        return Complex64::new(b / d, -a / d);
    }
    let cd = c.abs().max(d.abs());
    let ab = a.abs().max(b.abs());
    let safe = f64::MIN_POSITIVE.sqrt()..=f64::MAX.sqrt() * 0.5;
    if !cd.is_finite()
        || !ab.is_finite()
        || (safe.contains(&cd) && (ab == 0. || safe.contains(&ab)))
    {
        return cxl_smith(a, b, c, d);
    }
    let ec = libm::ilogb(cd);
    let ea = if ab == 0. { 0 } else { libm::ilogb(ab) };
    let z = cxl_smith(
        libm::scalbn(a, -ea),
        libm::scalbn(b, -ea),
        libm::scalbn(c, -ec),
        libm::scalbn(d, -ec),
    );
    Complex64::new(libm::scalbn(z.re, ea - ec), libm::scalbn(z.im, ea - ec))
}

impl Engine {
    /// Triangular solve in ztrsm's column order. Division follows the scalar
    /// operator, so a zero pivot gives the reference's values (`[0 0; 0 0]\[1i; 1]` is
    /// `[NaN+Infi; NaN+NaNi]`). Cancellation is checked per right-hand-side
    /// column and every `CANCEL_STEPS` steps.
    fn cxl_substitute(&self, t: &CMat, b: &CMat, lower: bool) -> Result<CMat> {
        let n = t.nrows();
        let mut x = b.clone();
        for c in 0..x.ncols() {
            for step in 0..n {
                if step % CANCEL_STEPS == 0 {
                    self.check_cancel()?;
                }
                let k = if lower { step } else { n - 1 - step };
                let v = cxl_scalar_divide(x[(k, c)], t[(k, k)]);
                x[(k, c)] = v;
                let rows = if lower { k + 1..n } else { 0..k };
                for i in rows {
                    x[(i, c)] -= v * t[(i, k)];
                }
            }
        }
        Ok(x)
    }

    /// U\(L\(P*b)) of a factor with an exact zero pivot, as the reference's solve
    /// still returns it: a zero pivot turns its component into NaN.
    fn cxl_singular_solve(
        &self,
        lu: &nalgebra::linalg::LU<Complex64, nalgebra::Dyn, nalgebra::Dyn>,
        b: &CMat,
    ) -> Result<CMat> {
        let (l, u) = (lu.l(), lu.u());
        let n = u.nrows();
        let mut x = b.clone();
        lu.p().permute_rows(&mut x);
        for c in 0..x.ncols() {
            for k in 0..n {
                if k % CANCEL_STEPS == 0 {
                    self.check_cancel()?;
                }
                let v = x[(k, c)];
                for i in k + 1..n {
                    x[(i, c)] -= v * l[(i, k)];
                }
            }
            for k in (0..n).rev() {
                if k % CANCEL_STEPS == 0 {
                    self.check_cancel()?;
                }
                let v = if u[(k, k)] == ZERO {
                    Complex64::new(f64::NAN, f64::NAN)
                } else {
                    x[(k, c)] / u[(k, k)]
                };
                x[(k, c)] = v;
                for i in 0..k {
                    x[(i, c)] -= v * u[(i, k)];
                }
            }
        }
        Ok(x)
    }

    /// The operand classes complex division accepts.
    fn cxl_operands(&self, operands: [&Array; 2]) -> Result<()> {
        for operand in operands {
            match operand {
                Array::Complex(_) | Array::Numeric(_) | Array::Logical(_) => {}
                Array::Typed(_) => {
                    return Err(
                        "Complex division supports double and logical operands only; integer and single complex values are not supported yet".into(),
                    );
                }
                _ => return Err("Complex division requires numeric or logical operands".into()),
            }
        }
        Ok(())
    }

    /// `a / b` (or `a \ b` when `backslash`) with a scalar divisor, after the
    /// caller's shape, budget and cancellation checks.
    pub(super) fn cxl_divide(&self, backslash: bool, a: &Array, b: &Array) -> Result<Value> {
        let (num, den) = if backslash { (b, a) } else { (a, b) };
        self.cxl_operands([num, den])?;
        let d = cxl_element(den, 0);
        let mut data = Vec::new();
        data.try_reserve_exact(num.len())
            .map_err(|_| "Cannot allocate complex division output")?;
        for first in (0..num.len()).step_by(4096) {
            self.check_cancel()?;
            for i in first..(first + 4096).min(num.len()) {
                data.push(cxl_scalar_divide(cxl_element(num, i), d));
            }
        }
        let result = ComplexMatrix::from_column_vec(num.rows(), num.cols(), data, &self.cancel)?;
        // As complex_arithmetic does: all-zero imaginary parts narrow to real.
        let mut all_real = !result.is_empty();
        for chunk in result.data().chunks(4096) {
            self.check_cancel()?;
            if !chunk.iter().all(|z| z.im == 0.) {
                all_real = false;
                break;
            }
        }
        if all_real {
            return Ok(Value::Matrix(result.real(&self.cancel)?));
        }
        Ok(Value::Complex(result))
    }

    /// Operand shapes of `a / b` or `a \ b` when the divisor is not a scalar:
    /// a size mismatch is MATLAB's error; rectangular solves are not implemented here.
    pub(super) fn cxl_solve_shape(&self, backslash: bool, a: &Array, b: &Array) -> String {
        let agree = if backslash {
            a.rows() == b.rows()
        } else {
            a.cols() == b.cols()
        };
        if agree {
            "Complex arithmetic currently supports +, -, .*, ./ and *; complex powers and linear solves are not implemented".into()
        } else {
            DIMENSIONS.into()
        }
    }

    /// `a \ b` or `a / b` with a non-scalar divisor: square systems by LU
    /// (triangular divisors by substitution) with the reference's warnings.
    pub(super) fn cxl_solve(
        &mut self,
        backslash: bool,
        a: &Array,
        b: &Array,
        out: &mut Execution,
    ) -> Result<Value> {
        let divisor = if backslash { a } else { b };
        let other = if backslash { b } else { a };
        let n = divisor.rows();
        let agree = if backslash {
            other.rows() == n
        } else {
            other.cols() == n
        };
        if divisor.cols() != n || !agree {
            return Err(self.cxl_solve_shape(backslash, a, b));
        }
        self.cxl_operands([a, b])?;
        let k = if backslash {
            other.cols()
        } else {
            other.rows()
        };
        self.cxl_work(n, n)?;
        self.cxl_reserve(&[(n, n), (n, k), (n, k)])?;
        let (mut t, mut rhs) = (cxl_dense(divisor), cxl_dense(other));
        if !backslash {
            // b/A = (A.' \ b.').'
            t.transpose_mut();
            rhs = rhs.transpose();
        }
        self.check_cancel()?;
        let x = self.cxl_mldivide(&t, &rhs, out)?;
        self.cxl_value(&if backslash { x } else { x.transpose() })
    }

    /// Square `A\B` as the reference chooses: substitution for a lower (checked
    /// first) or upper triangular A, otherwise LU with partial pivoting.
    fn cxl_mldivide(&mut self, a: &CMat, b: &CMat, out: &mut Execution) -> Result<CMat> {
        let n = a.nrows();
        if n == 0 {
            return Ok(CMat::zeros(0, b.ncols()));
        }
        let lower = (0..n).all(|j| (0..j).all(|i| a[(i, j)] == ZERO));
        let upper = (0..n).all(|j| (j + 1..n).all(|i| a[(i, j)] == ZERO));
        if !(lower || upper) {
            return self.cxl_lu_solve(a, b, out);
        }
        self.check_cancel()?;
        let x = self.cxl_substitute(a, b, lower)?;
        if (0..n).any(|i| a[(i, i)] == ZERO) {
            self.lax_singular_warning(out)?;
        } else {
            let adjoint = a.adjoint();
            let estimate = if a.iter().all(cxl_finite) {
                self.cxl_inverse_norm1(n, |v, ad| {
                    Ok(Some(if ad {
                        self.cxl_substitute(&adjoint, v, !lower)?
                    } else {
                        self.cxl_substitute(a, v, lower)?
                    }))
                })?
            } else {
                Some(f64::NAN)
            };
            self.cxl_condition_warning(cxl_norm1(a), estimate, out)?;
        }
        Ok(x)
    }

    /// LU solve (zgesv) with the zgecon warnings; an exact zero pivot warns
    /// that A is singular and still substitutes, as the reference does.
    fn cxl_lu_solve(&mut self, a: &CMat, b: &CMat, out: &mut Execution) -> Result<CMat> {
        let lu = a.clone().lu();
        self.check_cancel()?;
        let Some(x) = lu.solve(b) else {
            self.lax_singular_warning(out)?;
            return self.cxl_singular_solve(&lu, b);
        };
        self.cxl_lu_warning(a, &lu, out)?;
        Ok(x)
    }

    /// zgecon's RCOND warning for a nonsingular LU factor of A.
    fn cxl_lu_warning(
        &mut self,
        a: &CMat,
        lu: &nalgebra::linalg::LU<Complex64, nalgebra::Dyn, nalgebra::Dyn>,
        out: &mut Execution,
    ) -> Result<()> {
        let estimate = if a.iter().all(cxl_finite) {
            // P*A = L*U, so A' y = c is U' (L' (P y)) = c.
            let (l, u, p) = (lu.l(), lu.u(), lu.p());
            let adjoint_solve = |v: &CMat| {
                let mut y = l.ad_solve_lower_triangular(&u.ad_solve_upper_triangular(v)?)?;
                p.inv_permute_rows(&mut y);
                Some(y)
            };
            self.cxl_inverse_norm1(a.nrows(), |v, adjoint| {
                Ok(if adjoint {
                    adjoint_solve(v)
                } else {
                    lu.solve(v)
                })
            })?
        } else {
            Some(f64::NAN)
        };
        self.cxl_condition_warning(cxl_norm1(a), estimate, out)
    }

    /// LAPACK zlacn2 (Hager/Higham, complex) estimate of norm(inv(A),1) from
    /// solves with A (`false`) and A' (`true`): x/|x| replaces dlacn2's signs.
    fn cxl_inverse_norm1(
        &self,
        n: usize,
        solve: impl Fn(&CMat, bool) -> Result<Option<CMat>>,
    ) -> Result<Option<f64>> {
        if n == 0 {
            return Ok(Some(0.));
        }
        let phase = |x: &CMat| {
            x.map(|z| {
                let a = z.norm();
                if a > f64::MIN_POSITIVE {
                    Complex64::new(z.re / a, z.im / a)
                } else {
                    ONE
                }
            })
        };
        let asum = |x: &CMat| x.iter().map(|z| z.norm()).sum::<f64>();
        let start = CMat::from_element(n, 1, Complex64::new(1. / n as f64, 0.));
        let Some(x) = solve(&start, false)? else {
            return Ok(None);
        };
        if n == 1 {
            return Ok(Some(x[0].norm()));
        }
        let mut estimate = asum(&x);
        let Some(x) = solve(&phase(&x), true)? else {
            return Ok(None);
        };
        let mut j = cxl_first_max(&x);
        for iteration in 2.. {
            self.check_cancel()?;
            let mut unit = CMat::zeros(n, 1);
            unit[j] = ONE;
            let Some(x) = solve(&unit, false)? else {
                return Ok(None);
            };
            let old = estimate;
            estimate = asum(&x);
            // A smaller estimate cycles.
            if estimate <= old {
                break;
            }
            let Some(x) = solve(&phase(&x), true)? else {
                return Ok(None);
            };
            let last = j;
            j = cxl_first_max(&x);
            if x[last].norm() == x[j].norm() || iteration >= 5 {
                break;
            }
        }
        let alternating = CMat::from_fn(n, 1, |i, _| {
            Complex64::new(
                (1. + i as f64 / (n - 1) as f64) * if i % 2 == 0 { 1. } else { -1. },
                0.,
            )
        });
        let Some(x) = solve(&alternating, false)? else {
            return Ok(None);
        };
        Ok(Some(estimate.max(2. * asum(&x) / (3 * n) as f64)))
    }

    /// The RCOND warning of a nonsingular solve; a NaN estimate (non-finite
    /// input) gives the reference's other wording.
    fn cxl_condition_warning(
        &mut self,
        norm: f64,
        inverse_norm: Option<f64>,
        out: &mut Execution,
    ) -> Result<()> {
        let rcond = match inverse_norm {
            Some(e) if e.is_nan() || norm.is_nan() => f64::NAN,
            Some(e) if norm != 0. && e != 0. => (1. / e) / norm,
            _ => 0.,
        };
        self.lax_rcond_warning(rcond, out)
    }

    /// mldivide / mrdivide called as functions with a complex argument.
    pub(super) fn cxl_divide_call(
        &mut self,
        backslash: bool,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Value> {
        let a = args[0]
            .clone()
            .numeric_operand(&self.cancel)?
            .array(&self.cancel)?;
        let b = args[1]
            .clone()
            .numeric_operand(&self.cancel)?
            .array(&self.cancel)?;
        let divisor = if backslash { &a } else { &b };
        if divisor.len() != 1 {
            return self.cxl_solve(backslash, &a, &b, out);
        }
        let (rows, cols) = if backslash {
            (b.rows(), b.cols())
        } else {
            (a.rows(), a.cols())
        };
        let units = ComplexMatrix::checked_len(rows, cols)? * 2;
        if self.evaluated_elements.saturating_add(units) > BUDGET {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        self.cxl_divide(backslash, &a, &b)
    }

    /// inv of a complex matrix: LU solve of the identity; a singular matrix
    /// warns and gives Inf, as the reference does.
    pub(super) fn cxl_inv(&mut self, value: &Value, out: &mut Execution) -> Result<Value> {
        let a = self.cxl_square(value)?;
        let n = a.nrows();
        self.cxl_work(n, n)?;
        self.cxl_reserve(&[(n, n), (n, n), (n, n)])?;
        if n == 0 {
            return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
        }
        if n == 1 && cxl_finite(&a[0]) && a[0] != ZERO {
            // As 1/z: nalgebra's complex division squares the components.
            return self.cxl_value(&CMat::from_element(1, 1, cxl_scalar_divide(ONE, a[0])));
        }
        let lu = a.clone().lu();
        self.check_cancel()?;
        let Some(x) = lu.solve(&CMat::identity(n, n)) else {
            self.lax_singular_warning(out)?;
            return Ok(Value::Matrix(Matrix::from_column_vec(
                n,
                n,
                vec![f64::INFINITY; n * n],
            )?));
        };
        self.cxl_lu_warning(&a, &lu, out)?;
        self.cxl_value(&x)
    }

    /// det of a complex matrix: the product of U's diagonal with the pivot
    /// sign; a zero imaginary part narrows to real (Hermitian input).
    pub(super) fn cxl_det(&self, value: &Value) -> Result<Value> {
        let a = self.cxl_square(value)?;
        let n = a.nrows();
        self.cxl_work(n, n)?;
        self.cxl_reserve(&[(n, n)])?;
        let d = if n == 0 { ONE } else { a.lu().determinant() };
        self.check_cancel()?;
        self.cxl_value(&CMat::from_element(1, 1, d))
    }

    /// lu of a complex matrix: Y, [L, U] or [L, U, P] (P a row vector with
    /// 'vector'), as linalg2_lu; nalgebra pivots by |re|+|im| like izamax.
    pub(super) fn cxl_lu(&self, args: &[Value], outputs: usize) -> Result<Vec<Value>> {
        let vector = match args.get(1) {
            None => false,
            Some(option) => {
                let text = match option {
                    Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
                    Value::Container(Array::Character(a)) => Some(a.to_utf8(&self.cancel)?),
                    _ => None,
                };
                match text.as_deref() {
                    Some("vector") => true,
                    Some("matrix") => false,
                    _ => return Err("Unknown lu option: use 'matrix' or 'vector'.".into()),
                }
            }
        };
        let a = args[0]
            .clone()
            .numeric_operand(&self.cancel)?
            .array(&self.cancel)?;
        self.cxl_operands([&a, &a])?;
        let (m, n) = (a.rows(), a.cols());
        let k = m.min(n);
        self.cxl_work(m, n)?;
        let p_shape = if vector { (1, m) } else { (m, m) };
        self.cxl_reserve(&match outputs {
            1 => vec![(m, n), (m, n)],
            2 => vec![(m, n), (m, k), (k, n)],
            _ => vec![(m, n), (m, k), (k, n), p_shape],
        })?;
        let perm = |order: &[usize]| -> Result<Value> {
            if vector {
                let v = order.iter().map(|&i| i as f64 + 1.).collect();
                return Ok(Value::Matrix(Matrix::from_column_vec(1, m, v)?));
            }
            let mut p = vec![0.; m * m];
            for (i, &j) in order.iter().enumerate() {
                p[i + j * m] = 1.;
            }
            Ok(Value::Matrix(Matrix::from_column_vec(m, m, p)?))
        };
        if k == 0 {
            let identity: Vec<usize> = (0..m).collect();
            return Ok(match outputs {
                1 => vec![self.cxl_value(&CMat::zeros(m, n))?],
                2 => vec![
                    self.cxl_value(&CMat::zeros(m, k))?,
                    self.cxl_value(&CMat::zeros(k, n))?,
                ],
                _ => vec![
                    self.cxl_value(&CMat::zeros(m, k))?,
                    self.cxl_value(&CMat::zeros(k, n))?,
                    perm(&identity)?,
                ],
            });
        }
        let lu = cxl_dense(&a).lu();
        self.check_cancel()?;
        let (mut l, u) = (lu.l(), lu.u());
        // order[i] = original row now at row i, so that P*A = L*U with P = I(order,:)
        let mut idx = DMatrix::from_fn(m, 1, |i, _| i as f64);
        lu.p().permute_rows(&mut idx);
        let order: Vec<usize> = idx.iter().map(|&v| v as usize).collect();
        Ok(match outputs {
            1 => {
                let y = CMat::from_fn(m, n, |i, j| if i <= j { u[(i, j)] } else { l[(i, j)] });
                vec![self.cxl_value(&y)?]
            }
            2 => {
                lu.p().inv_permute_rows(&mut l);
                vec![self.cxl_value(&l)?, self.cxl_value(&u)?]
            }
            _ => vec![self.cxl_value(&l)?, self.cxl_value(&u)?, perm(&order)?],
        })
    }

    /// The square complex operand of inv and det.
    fn cxl_square(&self, value: &Value) -> Result<CMat> {
        let a = value
            .clone()
            .numeric_operand(&self.cancel)?
            .array(&self.cancel)?;
        self.cxl_operands([&a, &a])?;
        if a.rows() != a.cols() {
            return Err(SQUARE.into());
        }
        Ok(cxl_dense(&a))
    }

    fn cxl_work(&self, m: usize, n: usize) -> Result<()> {
        if (m as f64) * (n as f64) * (m.min(n) as f64) > MAX_WORK {
            return Err("The factorization exceeds the dense work limit of this version".into());
        }
        Ok(())
    }

    /// Complex work buffers and results (two units per element) against the
    /// statement budget, before anything of that size is allocated.
    fn cxl_reserve(&self, shapes: &[(usize, usize)]) -> Result<()> {
        let mut total = self.evaluated_elements;
        for &(rows, cols) in shapes {
            total = total.saturating_add(ComplexMatrix::checked_len(rows, cols)?.saturating_mul(2));
        }
        if total > BUDGET {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }

    /// A complex result as a Value: all-zero imaginary parts (or an empty
    /// result) narrow to real, as the solve and factor results do in the reference.
    fn cxl_value(&self, x: &CMat) -> Result<Value> {
        self.check_cancel()?;
        if x.iter().all(|z| z.im == 0.) {
            let re = x.iter().map(|z| z.re).collect();
            return Ok(Value::Matrix(Matrix::from_column_vec(
                x.nrows(),
                x.ncols(),
                re,
            )?));
        }
        Ok(Value::Complex(ComplexMatrix::from_column_vec(
            x.nrows(),
            x.ncols(),
            x.as_slice().to_vec(),
            &self.cancel,
        )?))
    }
}
