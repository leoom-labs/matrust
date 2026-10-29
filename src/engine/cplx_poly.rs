//! Complex input to polyval, polyfit, conv, deconv, filter, filtfilt, interp1,
//! trapz, cumtrapz, roots, poly and polyder (reference semantics, cplx3 corpus).
//! The existing real functions route here when an argument is complex; private
//! methods use the `cxp_` prefix. Results whose imaginary parts are all zero
//! narrow to real, as the reference's arithmetic does.
use super::eig_more::{
    cdense::{self, CMat, cdiv},
    cgen,
    dense::Mat,
    rgen,
};
use super::poly_interp::{complex_value, matrix_value, narrowed_value, scalar_of, text_of};
use super::{Engine, Execution, Fma, Value};
use crate::{Complex64 as C, Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const NAMES: &[&str] = &[
    "polyval", "polyfit", "conv", "deconv", "filter", "filtfilt", "interp1", "trapz", "cumtrapz",
    "roots", "poly", "polyder",
];
const ZERO: C = C { re: 0., im: 0. };
const ONE: C = C { re: 1., im: 0. };
/// Largest filter or polynomial order handled here (as the real kernels).
const MAX_ORDER: usize = 4096;
/// Largest convolution (products), as the real conv.
const MAX_WORK: u128 = 1 << 36;
/// Products at which conv and polyval split their outputs over the rayon pool
/// (benchmarked in tool/bench-cplx-conv.sh).
const PARALLEL_WORK: u128 = 1 << 18;
/// Outputs per cancellation check and per parallel task.
const CHUNK: usize = 256;
/// Largest roots degree and poly matrix size, as the real roots and poly.
const MAX_DEGREE: usize = 1000;
const SINGULAR_STATE: &str =
    "filtfilt has singular steady-state initial conditions (for example a pole at DC)";
const NONFINITE_STATE: &str = "filtfilt initial-state solve is nonfinite";
const COMPLEX_POINTS: &str = "Data points in complex number format are not supported.\nUse REAL and IMAG to extract the real and imaginary components.";

/// True when `name` is one of these functions and some argument is complex.
pub(super) fn routes(name: &str, args: &[Value]) -> bool {
    NAMES.contains(&name) && args.iter().any(|v| matches!(v, Value::Complex(_)))
}

/// True when some argument is complex.
pub(super) fn complex_input(args: &[Value]) -> bool {
    args.iter().any(|v| matches!(v, Value::Complex(_)))
}

/// (rows, cols, column-major data) of a double, logical or complex array.
fn parts(value: &Value, who: &str) -> Result<(usize, usize, Vec<C>)> {
    match value {
        Value::Complex(c) => Ok((c.rows(), c.cols(), c.data().to_vec())),
        Value::Matrix(m) => Ok((
            m.rows(),
            m.cols(),
            m.data().iter().map(|&x| C::new(x, 0.)).collect(),
        )),
        Value::Logical(l) => Ok((
            l.rows(),
            l.cols(),
            l.data().iter().map(|&b| C::new(f64::from(b), 0.)).collect(),
        )),
        _ => Err(format!(
            "{who}: complex input must be combined with double or logical arrays"
        )),
    }
}
fn is_vector(rows: usize, cols: usize) -> bool {
    rows == 1 || cols == 1
}
fn finite(z: &C) -> bool {
    z.re.is_finite() && z.im.is_finite()
}
/// Products as the reference forms them: a real factor scales both parts.
#[inline]
fn mul(a: C, b: C) -> C {
    if a.im == 0. {
        C::new(a.re * b.re, a.re * b.im)
    } else if b.im == 0. {
        C::new(a.re * b.re, a.im * b.re)
    } else {
        a * b
    }
}
/// Division as `./`: a real divisor divides both parts.
#[inline]
fn div(a: C, b: C) -> C {
    if b.im == 0. {
        C::new(a.re / b.re, a.im / b.re)
    } else {
        cdiv(a, b)
    }
}
/// 2 end - sample, each part with one rounding (libm fma), so the result is
/// found whenever it is representable even if 2 end alone is not.
fn reflect(end: C, sample: C) -> C {
    C::new(end.re.fma(2., -sample.re), end.im.fma(2., -sample.im))
}
fn real_part(data: &[C]) -> Vec<f64> {
    data.iter().map(|z| z.re).collect()
}
fn imag_part(data: &[C]) -> Vec<f64> {
    data.iter().map(|z| z.im).collect()
}
fn split(value: &Value, who: &str) -> Result<(Value, Value)> {
    let (r, c, data) = parts(value, who)?;
    Ok((
        Value::Matrix(Matrix::from_column_vec(r, c, real_part(&data))?),
        Value::Matrix(Matrix::from_column_vec(r, c, imag_part(&data))?),
    ))
}
fn joined(re: &Value, im: &Value) -> Result<(usize, usize, Vec<C>)> {
    let (Value::Matrix(re), Value::Matrix(im)) = (re, im) else {
        return Err("internal error: expected real parts".into());
    };
    Ok((
        re.rows(),
        re.cols(),
        re.data()
            .iter()
            .zip(im.data())
            .map(|(&a, &b)| C::new(a, b))
            .collect(),
    ))
}
fn arg_count(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        Err("Not enough input arguments.".into())
    } else if args.len() > max {
        Err("Too many input arguments.".into())
    } else {
        Ok(())
    }
}
/// Fills `out[k] = term(k)`, checking for cancellation every CHUNK outputs.
/// `cost` counts the products; at PARALLEL_WORK and above the chunks run on the
/// rayon pool. Every output is still summed by one task in a fixed order, so
/// the result does not depend on the number of workers.
fn fill(
    out: &mut [C],
    cost: u128,
    engine: &Engine,
    term: impl Fn(usize) -> C + Sync,
) -> Result<()> {
    let cancel: &AtomicBool = &engine.cancel;
    let chunk = |(i, part): (usize, &mut [C])| -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Calculation cancelled".into());
        }
        for (k, slot) in part.iter_mut().enumerate() {
            *slot = term(i * CHUNK + k);
        }
        Ok(())
    };
    if cost >= PARALLEL_WORK && out.len() > CHUNK {
        out.par_chunks_mut(CHUNK).enumerate().try_for_each(chunk)
    } else {
        out.chunks_mut(CHUNK).enumerate().try_for_each(chunk)
    }
}
/// The convolution sum c(k) = sum_j a(j) b(k - j) (Oppenheim & Schafer,
/// Discrete-Time Signal Processing, 3rd ed., 2010, §2.3), each output summed
/// over increasing j. An empty factor gives zeros of the other length.
fn convolve(a: &[C], b: &[C], engine: &Engine) -> Result<Vec<C>> {
    if a.is_empty() || b.is_empty() {
        return Ok(vec![ZERO; a.len().max(b.len())]);
    }
    let cost = a.len() as u128 * b.len() as u128;
    if cost > MAX_WORK {
        return Err("Convolution is too large (more than 2^36 products)".into());
    }
    let mut c = vec![ZERO; a.len() + b.len() - 1];
    fill(&mut c, cost, engine, |k| {
        let first = k.saturating_sub(b.len() - 1);
        let last = k.min(a.len() - 1);
        (first..=last).fold(ZERO, |sum, j| sum + mul(a[j], b[k - j]))
    })?;
    Ok(c)
}
/// True when the values off the real axis pair up exactly with their complex
/// conjugates (as multisets), so that the product of the linear factors
/// (z - e(k)) has real coefficients.
fn conjugate_closed(e: &[C]) -> bool {
    let order = |x: &C, y: &C| x.re.total_cmp(&y.re).then(x.im.total_cmp(&y.im));
    let mut upper: Vec<C> = e.iter().filter(|z| z.im > 0.).copied().collect();
    let mut lower: Vec<C> = e.iter().filter(|z| z.im < 0.).map(|z| z.conj()).collect();
    if upper.len() != lower.len() {
        return false;
    }
    upper.sort_by(order);
    lower.sort_by(order);
    upper == lower
}
/// Coefficients of the derivative of p (descending powers): the term
/// p(k) z^(n-1-k) gives (n-1-k) p(k) z^(n-2-k) (Burden & Faires, Numerical
/// Analysis, §2.6; NIST DLMF §3.3). A constant has derivative 0.
fn derivative(p: &[C]) -> Vec<C> {
    let n = p.len();
    if n < 2 {
        return vec![ZERO];
    }
    p[..n - 1]
        .iter()
        .enumerate()
        .map(|(k, &c)| mul(C::new((n - 1 - k) as f64, 0.), c))
        .collect()
}
/// x + y, or x - y, with the coefficient vectors aligned at the constant term.
fn aligned_sum(x: Vec<C>, y: Vec<C>, subtract: bool) -> Vec<C> {
    let n = x.len().max(y.len());
    let mut sum = vec![ZERO; n];
    sum[n - x.len()..].copy_from_slice(&x);
    for (to, v) in sum[n - y.len()..].iter_mut().zip(y) {
        *to = if subtract { *to - v } else { *to + v };
    }
    sum
}
/// p without its leading zero coefficients; the zero polynomial keeps one 0.
fn without_leading_zeros(mut p: Vec<C>) -> Vec<C> {
    let lead = p.iter().position(|z| *z != ZERO).unwrap_or(p.len());
    p.drain(..lead);
    if p.is_empty() {
        p.push(ZERO);
    }
    p
}
/// True for a column vector (one column, not one row).
fn is_column(rows: usize, cols: usize) -> bool {
    cols == 1 && rows != 1
}

/// Normalized direct-form II transposed filter (filter's recurrence and
/// operation order); FIR filters skip the zero feedback terms.
struct Df2t {
    b: Vec<C>,
    a: Vec<C>,
    feedback: bool,
}
impl Df2t {
    fn new(b: &[C], a: &[C]) -> Result<Self> {
        let n = b.len().max(a.len());
        if n > MAX_ORDER + 1 {
            return Err(format!("filter supports order at most {MAX_ORDER}"));
        }
        let a0 = a[0];
        let mut nb = vec![ZERO; n];
        let mut na = vec![ZERO; n];
        for (to, &from) in nb.iter_mut().zip(b) {
            *to = if a0 == ONE { from } else { div(from, a0) };
        }
        for (to, &from) in na.iter_mut().zip(a) {
            *to = if a0 == ONE { from } else { div(from, a0) };
        }
        Ok(Self {
            feedback: na[1..].iter().any(|v| *v != ZERO),
            b: nb,
            a: na,
        })
    }
    fn order(&self) -> usize {
        self.b.len() - 1
    }
    /// Filter `x` in place from state `z` (length order), leaving the final state in `z`.
    fn run(&self, x: &mut [C], z: &mut [C], engine: &Engine) -> Result<()> {
        let n = z.len();
        for (k, value) in x.iter_mut().enumerate() {
            if k % 4096 == 0 {
                engine.check_cancel()?;
            }
            let input = *value;
            let y = mul(self.b[0], input) + z.first().copied().unwrap_or(ZERO);
            if n > 0 {
                for j in 0..n - 1 {
                    z[j] = if self.feedback {
                        z[j + 1] + mul(self.b[j + 1], input) - mul(self.a[j + 1], y)
                    } else {
                        z[j + 1] + mul(self.b[j + 1], input)
                    };
                }
                z[n - 1] = if self.feedback {
                    mul(self.b[n], input) - mul(self.a[n], y)
                } else {
                    mul(self.b[n], input)
                };
            }
            *value = y;
        }
        Ok(())
    }
}

/// Upper-triangular solve R x = c (dtrsv/ztrsv column order, fused updates).
fn back_substitute(r: &CMat, n: usize, x: &mut [C]) {
    for j in (0..n).rev() {
        x[j] = div(x[j], r.at(j, j));
        let t = -x[j];
        for (i, xi) in x.iter_mut().enumerate().take(j) {
            *xi = cdense::madd(*xi, t, r.at(i, j));
        }
    }
}
/// Basic least-squares solution of a wide m-by-n system (m < n) by QR with
/// column pivoting, as backslash solves R\(Q'*y) for an underdetermined fit.
fn pivoted_solve(a: &CMat, rhs: &[C], cancel: &dyn Fn() -> bool) -> Vec<C> {
    let (m, n) = (a.rows, a.cols);
    let mut a = a.clone();
    let mut b = rhs.to_vec();
    let mut perm: Vec<usize> = (0..n).collect();
    let k = m.min(n);
    for i in 0..k {
        if cancel() {
            break;
        }
        let norm = |a: &CMat, j: usize| cdense::znrm2((i..m).map(|r| &a.data[r + j * m]));
        let best = (i..n).fold(i, |best, j| {
            if norm(&a, j) > norm(&a, best) {
                j
            } else {
                best
            }
        });
        if best != i {
            a.swap_cols(i, best, 0, m);
            perm.swap(i, best);
        }
        let mut x: Vec<C> = (i + 1..m).map(|r| a.at(r, i)).collect();
        let (beta, tau) = cdense::zlarfg(a.at(i, i), &mut x);
        let mut v = vec![ONE];
        v.extend_from_slice(&x);
        a.set(i, i, beta);
        for r in i + 1..m {
            a.set(r, i, ZERO);
        }
        if i + 1 < n {
            cdense::zlarf_left(&mut a, &v, tau.conj(), i, i + 1, n - i - 1);
        }
        // Q^H b with the same reflector.
        let mut column = CMat {
            rows: m,
            cols: 1,
            data: b.clone(),
        };
        cdense::zlarf_left(&mut column, &v, tau.conj(), i, 0, 1);
        b = column.data;
    }
    let tol = n.max(m) as f64 * super::poly_interp::eps_of(cdense::cabs(a.at(0, 0)));
    let rank = (0..k)
        .take_while(|&i| cdense::cabs(a.at(i, i)) > tol)
        .count();
    let mut z = b[..rank].to_vec();
    back_substitute(&a, rank, &mut z);
    let mut out = vec![ZERO; n];
    for (i, v) in z.into_iter().enumerate() {
        out[perm[i]] = v;
    }
    out
}

impl Engine {
    /// Every output of `name` for complex input; `nout` outputs are requested.
    pub(super) fn cxp_call(
        &mut self,
        name: &str,
        args: &[Value],
        nout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let args = self.text_arguments(args.to_vec())?;
        let args = &args[..];
        let maximum = match name {
            "deconv" | "filter" | "polyder" => 2,
            _ => 1,
        };
        if nout > maximum {
            return Err("Too many output arguments.".into());
        }
        let one = |v: Result<Value>| v.map(|v| vec![v]);
        let outputs = match name {
            "polyval" => one(self.cxp_polyval(args)),
            "polyfit" => one(self.cxp_polyfit(args, out)),
            "conv" => one(self.cxp_conv(args, out)),
            "deconv" => self.cxp_deconv(args, nout),
            "filter" => self.cxp_filter(args, nout),
            "filtfilt" => one(self.cxp_filtfilt(args)),
            "interp1" => one(self.cxp_interp1(args, out)),
            "trapz" | "cumtrapz" => one(self.cxp_trapz(args, name == "cumtrapz")),
            "roots" => one(self.cxp_roots(args, out)),
            "poly" => one(self.cxp_poly(args)),
            _ => self.cxp_polyder(args, nout),
        }?;
        self.check_cancel()?;
        Ok(outputs)
    }

    /// residue with complex input, for any number of outputs: outputs whose
    /// imaginary parts are all zero become real, and the b of
    /// residue(r, p, k) keeps its leading zeros (reference:
    /// residue([1i; -1i], [1i; -1i], []) gives b = [0 -2]).
    pub(super) fn cxp_narrow(&self, args: &[Value], outputs: Vec<Value>) -> Result<Vec<Value>> {
        let mut outputs = outputs;
        if args.len() == 3
            && let Some(first) = outputs.first()
        {
            let poles = parts(&args[1], "residue")?.2.len();
            let k = parts(&args[2], "residue")?.2.len();
            // a = poly(p) has poles + 1 terms; b has length(a) - 1 (+ length(k) - 1 with a direct term).
            let length = if k == 0 { poles } else { poles + k };
            let (_, _, numerator) = parts(first, "residue")?;
            if numerator.len() < length {
                let mut padded = vec![ZERO; length - numerator.len()];
                padded.extend(numerator);
                outputs[0] = complex_value(self, 1, length, padded)?;
            }
        }
        outputs
            .into_iter()
            .map(|v| match v {
                Value::Complex(c) => narrowed_value(self, c.rows(), c.cols(), c.data().to_vec()),
                other => Ok(other),
            })
            .collect()
    }

    fn cxp_warning(&mut self, id: &str, message: &str, out: &mut Execution) -> Result<()> {
        self.lang_utils_call(
            "warning",
            &[Value::Text(id.into()), Value::Text(message.into())],
            out,
        )?;
        Ok(())
    }

    /// polyval(p, x) by Horner's rule, y = (..(p(1) x + p(2)) x + ..) x + p(n)
    /// (Burden & Faires, Numerical Analysis, §2.6; NIST DLMF §3.3), elementwise
    /// over x. An empty p gives zeros.
    fn cxp_polyval(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 2, 2)?;
        let (pr, pc, p) = parts(&args[0], "polyval")?;
        let (xr, xc, x) = parts(&args[1], "polyval")?;
        if !p.is_empty() && !is_vector(pr, pc) {
            return Err("P must be a vector.".into());
        }
        if p.len() > MAX_ORDER {
            return Err(format!("polyval supports at most {MAX_ORDER} coefficients"));
        }
        self.polyopt_admit(x.len())?;
        let mut y = vec![ZERO; x.len()];
        let (lead, rest) = p.split_first().map_or((ZERO, &[][..]), |(&l, r)| (l, r));
        fill(&mut y, x.len() as u128 * p.len() as u128, self, |k| {
            let t = x[k];
            rest.iter().fold(lead, |acc, &c| mul(t, acc) + c)
        })?;
        narrowed_value(self, xr, xc, y)
    }

    /// polyfit(x, y, n): least squares for the Vandermonde system V p = y,
    /// V(i, j) = x(i)^(n+1-j), by Householder QR (Golub & Van Loan, Matrix
    /// Computations, 4th ed., 2013, §5.3): Q^H y, then back substitution in R.
    /// A real x keeps V and Q real, applied to both parts of y. Fewer points
    /// than coefficients (with a warning) and a rank-deficient R give the
    /// basic solution of the column-pivoted QR.
    fn cxp_polyfit(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        arg_count(args, 3, 3)?;
        let (_, _, x) = parts(&args[0], "polyfit")?;
        let (_, _, y) = parts(&args[1], "polyfit")?;
        if !x.iter().chain(&y).all(finite) {
            return Err("polyfit: X and Y must be finite".into());
        }
        let degree = scalar_of(&args[2], "polyfit", "the degree")?;
        if !(degree >= 0. && degree.fract() == 0. && degree <= MAX_ORDER as f64) {
            return Err(format!(
                "polyfit: the degree must be a nonnegative integer scalar (at most {MAX_ORDER})"
            ));
        }
        if x.len() != y.len() {
            return Err("X and Y vectors must be the same size.".into());
        }
        let (m, n) = (x.len(), degree as usize + 1);
        self.polyopt_admit(m.saturating_mul(n).saturating_mul(2))?;
        if n > m {
            self.cxp_warning(
                "MATLAB:polyfit:PolyNotUnique",
                "Polynomial is not unique; degree >= number of data points.",
                out,
            )?;
        }
        if m == 0 {
            return matrix_value(1, n, vec![0.; n]);
        }
        let mut v = CMat::zeros(m, n);
        for (i, &t) in x.iter().enumerate() {
            let mut power = ONE;
            for j in (0..n).rev() {
                v.set(i, j, power);
                if j > 0 {
                    power = mul(t, power);
                }
            }
        }
        let cancel = || self.cancel.load(Ordering::Relaxed);
        let p = if n > m {
            None
        } else if x.iter().all(|z| z.im == 0.) {
            let mut qr = v.re();
            let tau = rgen::geqr2(&mut qr, 0, 0, m, n, &cancel);
            let mut parts = real_part(&y);
            parts.extend(imag_part(&y));
            let mut rhs = Mat::from_vec(m, 2, parts);
            rgen::orm2r_lt(&qr, 0, 0, m, n, &tau, &mut rhs, 0, 0, 2, &cancel);
            let c = (0..n).map(|i| C::new(rhs.at(i, 0), rhs.at(i, 1))).collect();
            Some((CMat::from_real(&qr), c))
        } else {
            let mut qr = v.clone();
            let tau = cgen::zgeqr2(&mut qr, 0, 0, m, n, &cancel);
            let mut rhs = CMat {
                rows: m,
                cols: 1,
                data: y.clone(),
            };
            cgen::zunm2r_lc(&qr, 0, 0, m, n, &tau, &mut rhs, 0, 0, 1, &cancel);
            rhs.data.truncate(n);
            Some((qr, rhs.data))
        }
        .and_then(|(r, mut c)| {
            let size = (0..n).map(|i| cdense::cabs(r.at(i, i))).fold(0., f64::max);
            let tol = m as f64 * super::poly_interp::eps_of(size);
            (0..n).all(|i| cdense::cabs(r.at(i, i)) > tol).then(|| {
                back_substitute(&r, n, &mut c);
                c
            })
        })
        .unwrap_or_else(|| pivoted_solve(&v, &y, &cancel));
        self.check_cancel()?;
        narrowed_value(self, 1, n, p)
    }

    /// conv(u, v[, shape]): the convolution sum, then its central part of
    /// length(u) ('same') or the part without zero padding ('valid').
    fn cxp_conv(&mut self, args: &[Value], _out: &mut Execution) -> Result<Value> {
        arg_count(args, 2, 3)?;
        let shape = match args.get(2) {
            None => "full".to_string(),
            Some(v) => match text_of(v).map(str::to_ascii_lowercase) {
                Some(s) if matches!(s.as_str(), "full" | "same" | "valid") => s,
                _ => return Err("Unknown shape parameter.".into()),
            },
        };
        if args[..2].iter().any(|v| matches!(v, Value::Logical(_))) {
            return Err("First and second arguments must be single or double.".into());
        }
        let (ur, uc, u) = parts(&args[0], "conv")?;
        let (vr, vc, v) = parts(&args[1], "conv")?;
        if !is_vector(ur, uc) || !is_vector(vr, vc) {
            return Err("A and B must be vectors.".into());
        }
        self.polyopt_admit((u.len() + v.len()).saturating_mul(2))?;
        let full = convolve(&u, &v, self)?;
        let (data, column) = match shape.as_str() {
            "full" => {
                let column = if u.len() > v.len() {
                    is_column(ur, uc)
                } else {
                    is_column(vr, vc)
                };
                (full, column)
            }
            "same" => {
                let start = v.len() / 2;
                (full[start..start + u.len()].to_vec(), is_column(ur, uc))
            }
            _ => {
                // An empty v leaves length(u) + 1 zeros, as the real conv.
                let length = (u.len() + 1).saturating_sub(v.len());
                let start = v.len().saturating_sub(1);
                let mut part: Vec<C> = full.iter().skip(start).take(length).copied().collect();
                part.resize(length, ZERO);
                (part, is_column(ur, uc))
            }
        };
        let n = data.len();
        if column {
            narrowed_value(self, n, 1, data)
        } else {
            narrowed_value(self, 1, n, data)
        }
    }

    /// The poly_interp implementation of `name` (it accepts complex data).
    fn cxp_real_family(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Value> {
        self.polyopt_call(name, args, out)?
            .ok_or_else(|| format!("internal error: {name} is not in the poly_interp family"))
    }

    /// [q, r] = deconv(v, u) by polynomial long division (Oppenheim & Schafer,
    /// Discrete-Time Signal Processing, 3rd ed., §6.3; Burden & Faires §2.6):
    /// q(k) = r(k)/u(1), then r(k:k+n-1) -= q(k) u, so v = conv(u, q) + r and
    /// the leading length(q) entries of r are zero.
    fn cxp_deconv(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        arg_count(args, 2, 2)?;
        let (vr, vc, v) = parts(&args[0], "deconv")?;
        let (ur, uc, u) = parts(&args[1], "deconv")?;
        if !is_vector(vr, vc) || !is_vector(ur, uc) {
            return Err("deconv: A and B must be vectors.".into());
        }
        let Some(&lead) = u.first() else {
            return Err("Index exceeds matrix dimensions.".into());
        };
        if lead == ZERO {
            return Err("First coefficient of A must be non-zero.".into());
        }
        self.polyopt_admit(v.len().saturating_mul(4))?;
        let mut r = v;
        let q = if u.len() > r.len() {
            vec![ZERO]
        } else {
            // The division costs (length(v) - length(u) + 1) * length(u) products;
            // the filter order bound (as filter and filtfilt) keeps it small and
            // rejects huge operands before any work.
            if r.len() > MAX_ORDER + 1 {
                return Err(format!("filter supports order at most {MAX_ORDER}"));
            }
            let count = r.len() - u.len() + 1;
            let mut q = Vec::with_capacity(count);
            for k in 0..count {
                if k % CHUNK == 0 {
                    self.check_cancel()?;
                }
                let t = div(r[k], lead);
                r[k] = ZERO;
                for (slot, &c) in r[k + 1..].iter_mut().zip(&u[1..]) {
                    *slot -= mul(t, c);
                }
                q.push(t);
            }
            q
        };
        let n = q.len();
        let mut outputs = vec![if is_column(vr, vc) {
            narrowed_value(self, n, 1, q)?
        } else {
            narrowed_value(self, 1, n, q)?
        }];
        if nout > 1 {
            outputs.push(narrowed_value(self, vr, vc, r)?);
        }
        Ok(outputs)
    }

    /// filter(b, a, x[, zi[, dim]]) in complex arithmetic: y and the final state zf.
    fn cxp_filter(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        arg_count(args, 3, 5)?;
        let (br, bc, b) = parts(&args[0], "filter")?;
        let (ar, ac, a) = parts(&args[1], "filter")?;
        let (xr, xc, x) = parts(&args[2], "filter")?;
        if b.is_empty() || a.is_empty() || !is_vector(br, bc) || !is_vector(ar, ac) {
            return Err("filter: the coefficients must be nonempty vectors".into());
        }
        if a[0] == ZERO {
            return Err("First denominator filter coefficient must be non-zero.".into());
        }
        if !b.iter().chain(&a).all(finite) {
            return Err("filter: the coefficients must be finite".into());
        }
        let filter = Df2t::new(&b, &a)?;
        let order = filter.order();
        let dim = match args.get(4) {
            None => {
                if xr == 1 {
                    2
                } else {
                    1
                }
            }
            Some(Value::Matrix(m)) if m.len() == 1 && matches!(m.data()[0], 1. | 2.) => {
                m.data()[0] as usize
            }
            Some(_) => {
                return Err(
                    "filter dimension must be 1 or 2 for the current two-dimensional engine".into(),
                );
            }
        };
        let (samples, channels) = if dim == 1 { (xr, xc) } else { (xc, xr) };
        self.polyopt_admit(x.len() + order * channels)?;
        if (x.len() as u128) * (order as u128 + 1) > 1 << 36 {
            return Err("Filter operation exceeds work limit; process shorter chunks".into());
        }
        let initial = match args.get(3) {
            Some(v) => {
                let (zr, zc, z) = parts(v, "filter")?;
                let shared = z.len() == order && is_vector(zr, zc);
                if !z.is_empty() && !shared && (zr != order || zc != channels) {
                    return Err("Initial conditions must be a vector of length max(length(a),length(b))-1, or an array with the leading dimension of size max(length(a),length(b))-1 and with remaining dimensions matching those of x.".into());
                }
                (!z.is_empty()).then_some((z, shared))
            }
            None => None,
        };
        let mut y = x;
        let mut zf = vec![ZERO; order * channels];
        let index = |s: usize, ch: usize| {
            if dim == 1 { s + ch * xr } else { ch + s * xr }
        };
        let mut lane = vec![ZERO; samples];
        for ch in 0..channels {
            self.check_cancel()?;
            let state = &mut zf[ch * order..(ch + 1) * order];
            if let Some((z, shared)) = &initial {
                let start = if *shared { 0 } else { ch * order };
                state.copy_from_slice(&z[start..start + order]);
            }
            for (s, slot) in lane.iter_mut().enumerate() {
                *slot = y[index(s, ch)];
            }
            filter.run(&mut lane, state, self)?;
            for (s, &v) in lane.iter().enumerate() {
                y[index(s, ch)] = v;
            }
        }
        let mut outputs = vec![narrowed_value(self, xr, xc, y)?];
        if nout > 1 {
            outputs.push(narrowed_value(self, order, channels, zf)?);
        }
        Ok(outputs)
    }

    /// filtfilt(b, a, x): forward-backward filtering with the steady-state
    /// initial conditions of Gustafsson ("Determining the initial states in
    /// forward-backward filtering", IEEE Trans. Signal Processing 44, 1996),
    /// scaled by the first sample of each pass, on x extended at both ends by
    /// 3*order samples reflected through the end values (2 x(1) - x(k+1)).
    /// A row is filtered along the row, otherwise each column.
    fn cxp_filtfilt(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 3, 3)?;
        let (br, bc, b) = parts(&args[0], "filtfilt")?;
        let (ar, ac, a) = parts(&args[1], "filtfilt")?;
        let (xr, xc, x) = parts(&args[2], "filtfilt")?;
        if bc == 6 && (br > 1 || (br == 1 && b[3] == ONE && a.len() <= 2)) {
            return Err("filtfilt: SOS input must be real".into());
        }
        if b.is_empty() || a.is_empty() || !is_vector(br, bc) || !is_vector(ar, ac) {
            return Err("filtfilt: the coefficients must be nonempty vectors".into());
        }
        if a[0] == ZERO {
            return Err("First denominator filter coefficient must be non-zero.".into());
        }
        if !b.iter().chain(&a).all(finite) {
            return Err("filtfilt: the coefficients must be finite".into());
        }
        if !x.iter().all(finite) {
            return Err("filtfilt: the data must be finite".into());
        }
        let filter = Df2t::new(&b, &a)?;
        let order = filter.order();
        let edge = 3 * order;
        let (samples, channels) = if xr == 1 { (xc, 1) } else { (xr, xc) };
        if samples <= edge {
            return Err(format!("Data length must be larger than {edge} samples."));
        }
        let length = samples + 2 * edge;
        self.polyopt_admit((length * channels).saturating_add(x.len()))?;
        if (length as u128) * (channels as u128) * (order as u128 + 1) > MAX_WORK / 2 {
            return Err("Filter operation exceeds work limit; process shorter chunks".into());
        }
        let initial = steady_state(&filter.b, &filter.a)?;
        let mut y = x;
        let mut lane = vec![ZERO; length];
        let mut state = vec![ZERO; order];
        for channel in y.chunks_mut(samples) {
            self.check_cancel()?;
            let (first, last) = (channel[0], channel[samples - 1]);
            for k in 0..edge {
                lane[k] = reflect(first, channel[edge - k]);
                lane[edge + samples + k] = reflect(last, channel[samples - 2 - k]);
            }
            if !lane[..edge]
                .iter()
                .chain(&lane[edge + samples..])
                .all(finite)
            {
                return Err("filtfilt reflection overflowed".into());
            }
            lane[edge..edge + samples].copy_from_slice(channel);
            for pass in 0..2 {
                for (z, &s) in state.iter_mut().zip(&initial) {
                    *z = mul(s, lane[0]);
                }
                if !state.iter().all(finite) {
                    return Err("filtfilt scaled initial state overflowed".into());
                }
                // Filtering is causal: in the reversed lane the trailing
                // extension cannot reach the samples that are kept.
                let end = if pass == 0 { length } else { edge + samples };
                filter.run(&mut lane[..end], &mut state, self)?;
                if !lane[..end].iter().all(finite) {
                    return Err(if order == 0 {
                        "filtfilt gain overflowed"
                    } else {
                        "filtfilt recurrence overflowed; use a better-conditioned filter"
                    }
                    .into());
                }
                lane.reverse();
            }
            channel.copy_from_slice(&lane[edge..edge + samples]);
        }
        narrowed_value(self, xr, xc, y)
    }

    /// interp1 with complex samples: the real kernels interpolate the real and
    /// imaginary parts with the same method; complex x or xq is an error. As in
    /// the reference, the result stays complex unless a numeric fill value replaces
    /// some query outside the grid (that assignment narrows).
    fn cxp_interp1(&mut self, args: &[Value], out: &mut Execution) -> Result<Value> {
        let numeric = args.iter().take_while(|a| text_of(a).is_none()).count();
        if !(2..=3).contains(&numeric) {
            return self.cxp_real_family("interp1", args, out);
        }
        let complex = |v: &Value| matches!(v, Value::Complex(_));
        if complex(&args[numeric - 1]) || (numeric == 3 && complex(&args[0])) {
            return Err(COMPLEX_POINTS.into());
        }
        let v = numeric - 2;
        if numeric == 3 {
            let (vr, vc, _) = parts(&args[v], "interp1")?;
            let (_, _, x) = parts(&args[0], "interp1")?;
            let n = if is_vector(vr, vc) { vr * vc } else { vr };
            if x.len() != n {
                return Err(
                    "The grid vectors do not define a grid of points that match the given values."
                        .into(),
                );
            }
        }
        if args
            .get(numeric)
            .and_then(text_of)
            .is_some_and(|m| m.eq_ignore_ascii_case("cubic"))
        {
            self.cxp_warning(
                "MATLAB:interp1:UsePCHIP",
                "INTERP1(...,'CUBIC') will change in a future release. Use INTERP1(...,'PCHIP') instead.",
                out,
            )?;
        }
        let mut re_args = args.to_vec();
        let mut im_args = args.to_vec();
        let (re, im) = split(&args[v], "interp1")?;
        re_args[v] = re;
        im_args[v] = im;
        let mut filled = false;
        if let Some(fill) = args.get(numeric + 1)
            && text_of(fill).is_none()
        {
            let (re, im) = split(fill, "interp1")?;
            re_args[numeric + 1] = re;
            im_args[numeric + 1] = im;
            let grid = if numeric == 3 {
                real_part(&parts(&args[0], "interp1")?.2)
            } else {
                let (vr, vc, _) = parts(&args[v], "interp1")?;
                let n = if is_vector(vr, vc) { vr * vc } else { vr };
                (1..=n).map(|i| i as f64).collect()
            };
            let low = grid.iter().copied().fold(f64::INFINITY, f64::min);
            let high = grid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            filled = parts(&args[numeric - 1], "interp1")?
                .2
                .iter()
                .any(|q| q.re < low || q.re > high);
        }
        let re = self.cxp_real_family("interp1", &re_args, out)?;
        let im = self.cxp_real_family("interp1", &im_args, out)?;
        let (r, c, data) = joined(&re, &im)?;
        if filled {
            narrowed_value(self, r, c, data)
        } else {
            complex_value(self, r, c, data)
        }
    }

    /// Trapezoidal integration with a real spacing vector: the real kernel
    /// integrates the real and imaginary parts.
    fn cxp_trapz(&self, args: &[Value], cumulative: bool) -> Result<Value> {
        let who = if cumulative { "cumtrapz" } else { "trapz" };
        arg_count(args, 1, 3)?;
        let count = |v: &Value| parts(v, who).map(|p| p.2.len());
        let (x, y, dim) = match args.len() {
            1 => (None, &args[0], None),
            2 if count(&args[1])? == 1 => (None, &args[0], Some(&args[1])),
            2 => (Some(&args[0]), &args[1], None),
            _ => (Some(&args[0]), &args[1], Some(&args[2])),
        };
        let (rows, cols, _) = parts(y, who)?;
        let given = dim.is_some();
        let dim = match dim {
            None => {
                if rows != 1 || cols == 1 {
                    1
                } else {
                    2
                }
            }
            Some(Value::Matrix(m))
                if m.len() == 1
                    && m.data()[0] >= 1.
                    && m.data()[0].fract() == 0.
                    && m.data()[0] <= crate::matrix::MAX_ELEMENTS as f64 =>
            {
                m.data()[0] as usize
            }
            Some(_) => return Err("Dimension must be a bounded nonnegative integer".into()),
        };
        let x = match x {
            None => None,
            Some(Value::Matrix(m)) => Some(m.clone()),
            Some(_) => return Err(format!("{who}: complex X is not supported")),
        };
        if let Some(x) = &x {
            let length = match dim {
                1 => rows,
                2 => cols,
                _ => 1,
            };
            if x.len() != length {
                return Err(format!(
                    "LENGTH(X) must equal the length of Y in dim {dim}."
                ));
            }
        }
        let (re, im) = split(y, who)?;
        let integrate = |v: &Value| -> Result<Value> {
            let Value::Matrix(m) = v else {
                unreachable!("split returns real parts")
            };
            Ok(Value::Matrix(m.trapezoid(
                x.as_ref(),
                given.then_some(dim),
                cumulative,
                &self.cancel,
            )?))
        };
        let (r, c, data) = joined(&integrate(&re)?, &integrate(&im)?)?;
        narrowed_value(self, r, c, data)
    }

    /// roots(p): the eigenvalues of the companion matrix of p after leading
    /// zeros are dropped (Golub & Van Loan, Matrix Computations, 4th ed., 2013,
    /// §7.4.6; Edelman & Murakami, "Polynomial roots from companion matrix
    /// eigenvalues", Math. Comp. 64, 1995): first row -p(2:n+1)/p(1), ones on
    /// the subdiagonal. Each trailing zero coefficient adds a root at 0.
    fn cxp_roots(&mut self, args: &[Value], _out: &mut Execution) -> Result<Value> {
        arg_count(args, 1, 1)?;
        let (rows, cols, p) = parts(&args[0], "roots")?;
        if !p.is_empty() && !is_vector(rows, cols) {
            return Err("Input must be a vector.".into());
        }
        if !p.iter().all(finite) {
            return Err("Input to ROOTS must not contain NaN or Inf.".into());
        }
        let (Some(lead), Some(last)) = (
            p.iter().position(|z| *z != ZERO),
            p.iter().rposition(|z| *z != ZERO),
        ) else {
            return matrix_value(0, 1, Vec::new());
        };
        let c = &p[lead..=last];
        if c.len() > MAX_DEGREE + 1 {
            return Err(format!("roots supports degree up to {MAX_DEGREE}"));
        }
        let n = c.len() - 1;
        let mut values = if n == 0 {
            Vec::new()
        } else {
            let mut companion = CMat::zeros(n, n);
            for j in 0..n {
                let entry = -div(c[j + 1], c[0]);
                if !finite(&entry) {
                    return Err(
                        "roots: coefficients overflow when scaled by the leading term".into(),
                    );
                }
                companion.set(0, j, entry);
            }
            for i in 1..n {
                companion.set(i, i - 1, ONE);
            }
            self.cxp_eig(&companion)?
        };
        values.resize(values.len() + p.len() - 1 - last, ZERO);
        narrowed_value(self, values.len(), 1, values)
    }

    /// eig of a complex square matrix as MATLAB's eig chooses: zheev for a
    /// Hermitian matrix, otherwise zgeevx with balancing.
    fn cxp_eig(&self, a: &CMat) -> Result<Vec<C>> {
        let n = a.rows;
        let cancel = || self.cancel.load(std::sync::atomic::Ordering::Relaxed);
        if !a.data.iter().all(finite) {
            return Err("Input to EIG must not contain NaN or Inf.".into());
        }
        let hermitian = (0..n).all(|j| (0..=j).all(|i| a.at(i, j) == a.at(j, i).conj()));
        let values = if hermitian {
            let (d, _) = cdense::zheev(a, false, false, &cancel)
                .map_err(|_| "eig: the iteration did not converge".to_string())?;
            d.into_iter().map(|v| C::new(v, 0.)).collect()
        } else {
            cdense::zgeevx(a, true, false, false, &cancel)
                .map_err(|_| {
                    self.check_cancel()
                        .err()
                        .unwrap_or_else(|| "eig: the QR algorithm failed to converge".to_string())
                })?
                .w
        };
        self.check_cancel()?;
        Ok(values)
    }

    /// poly(r): the coefficients of the product of the linear factors (z - r(k)),
    /// one factor at a time, c(j) -= r(k) c(j-1) (Burden & Faires, Numerical
    /// Analysis, §2.6). For a square matrix r is its eigenvalues, so the result
    /// is det(zI - A). Roots closed under conjugation give real coefficients.
    fn cxp_poly(&self, args: &[Value]) -> Result<Value> {
        arg_count(args, 1, 1)?;
        let (rows, cols, data) = parts(&args[0], "poly")?;
        let roots = if data.is_empty() || is_vector(rows, cols) {
            data
        } else if rows == cols {
            if rows > MAX_DEGREE {
                return Err(format!(
                    "poly supports matrices up to {MAX_DEGREE}-by-{MAX_DEGREE}"
                ));
            }
            self.cxp_eig(&CMat { rows, cols, data })?
        } else {
            return Err("Argument must be a vector or a square matrix.".into());
        };
        let n = roots.len();
        if n > MAX_ORDER {
            return Err(format!("poly supports at most {MAX_ORDER} roots"));
        }
        let mut c = vec![ZERO; n + 1];
        c[0] = ONE;
        for (k, &r) in roots.iter().enumerate() {
            if k % CHUNK == 0 {
                self.check_cancel()?;
            }
            for j in (1..=k + 1).rev() {
                let previous = c[j - 1];
                c[j] -= mul(r, previous);
            }
        }
        if conjugate_closed(&roots) {
            matrix_value(1, n + 1, real_part(&c))
        } else {
            narrowed_value(self, 1, n + 1, c)
        }
    }

    /// meshgrid(x[, y]) with complex data (the 2-D form): X repeats x(:).' down
    /// the rows and Y repeats y(:) along the columns. A grid whose imaginary
    /// parts are all zero is real, as indexing narrows in the reference.
    pub(super) fn cxp_meshgrid(&self, args: &[Value], count: usize) -> Result<Vec<Value>> {
        let data = |value: &Value| -> Result<Vec<C>> {
            if value.is_character() {
                let codes = Value::Matrix(value.clone().matrix(&self.cancel)?);
                parts(&codes, "meshgrid").map(|p| p.2)
            } else {
                parts(value, "meshgrid").map(|p| p.2)
            }
        };
        let x = data(&args[0])?;
        let y = match args.get(1) {
            Some(second) => data(second)?,
            None => x.clone(),
        };
        let (rows, cols) = (y.len(), x.len());
        // Complex storage is twice the size of the real elements it is counted in.
        self.preflight_elements(
            Matrix::checked_len(rows, cols)?
                .saturating_mul(count.max(1))
                .saturating_mul(2),
        )?;
        let mut repeated = Vec::with_capacity(rows * cols);
        for &z in &x {
            self.check_cancel()?;
            repeated.extend(std::iter::repeat_n(z, rows));
        }
        let mut outputs = vec![narrowed_value(self, rows, cols, repeated)?];
        if count >= 2 {
            let mut tiled = Vec::with_capacity(rows * cols);
            for _ in 0..cols {
                self.check_cancel()?;
                tiled.extend_from_slice(&y);
            }
            outputs.push(narrowed_value(self, rows, cols, tiled)?);
        }
        Ok(outputs)
    }

    /// polyder(p), polyder(a, b) = (a b)' = a' b + a b', and
    /// [q, d] = polyder(b, a) for (b/a)' = (b' a - b a')/a^2 with d = a^2
    /// (product and quotient rules on coefficient vectors; Burden & Faires,
    /// Numerical Analysis, §2.6). Results are rows without leading zeros.
    fn cxp_polyder(&self, args: &[Value], nout: usize) -> Result<Vec<Value>> {
        arg_count(args, 1, 2)?;
        let first = parts(&args[0], "polyder")?.2;
        let row = |p: Vec<C>| {
            let p = without_leading_zeros(p);
            narrowed_value(self, 1, p.len(), p)
        };
        if args.len() == 1 && nout < 2 {
            return Ok(vec![row(derivative(&first))?]);
        }
        // [q, d] = polyder(b) is the quotient form with a = 1, as the real polyder.
        let second = match args.get(1) {
            Some(v) => parts(v, "polyder")?.2,
            None => vec![ONE],
        };
        self.polyopt_admit((first.len() + second.len()).saturating_mul(4))?;
        if nout < 2 {
            let left = convolve(&derivative(&first), &second, self)?;
            let right = convolve(&first, &derivative(&second), self)?;
            return Ok(vec![row(aligned_sum(left, right, false))?]);
        }
        let (b, a) = (first, second);
        let left = convolve(&derivative(&b), &a, self)?;
        let right = convolve(&b, &derivative(&a), self)?;
        let q = row(aligned_sum(left, right, true))?;
        Ok(vec![q, row(convolve(&a, &a, self)?)?])
    }
}

/// Steady-state initial conditions of the normalized transposed direct form II
/// filter (b and a of equal length n+1, a(1) = 1): the state z that a constant
/// unit input leaves unchanged. Its state-space equations (Oppenheim & Schafer,
/// Discrete-Time Signal Processing, 3rd ed., §6.5) with x = 1 and output
/// y = g = sum(b)/sum(a) give z(j) = sum over k > j of (b(k) - a(k) g),
/// 0-based. The system I - A has determinant sum(a), so it is singular exactly
/// when sum(a) = 0 (a pole at DC). When a sum, g or a product leaves the double
/// range the formula is evaluated on b and a divided by powers of two (z scales
/// with b and is unchanged by a common factor of a), then z is scaled back.
fn steady_state(b: &[C], a: &[C]) -> Result<Vec<C>> {
    if total(a) == ZERO {
        return Err(SINGULAR_STATE.into());
    }
    if let Some(z) = state_from_gain(b, a) {
        return Ok(z);
    }
    let (eb, ea) = (exponent_of(b), exponent_of(a));
    let Some(mut z) = state_from_gain(&scaled(b, -eb), &scaled(a, -ea)) else {
        return Err(NONFINITE_STATE.into());
    };
    for v in &mut z {
        *v = C::new(libm::scalbn(v.re, eb), libm::scalbn(v.im, eb));
    }
    if !z.iter().all(finite) {
        return Err(NONFINITE_STATE.into());
    }
    Ok(z)
}
fn total(v: &[C]) -> C {
    v.iter().fold(ZERO, |s, &x| s + x)
}
/// z(j) = sum over k > j of (b(k) - a(k) g) with g = sum(b)/sum(a); None when
/// some entry is not finite.
fn state_from_gain(b: &[C], a: &[C]) -> Option<Vec<C>> {
    let gain = div(total(b), total(a));
    let mut z = vec![ZERO; b.len() - 1];
    let mut tail = ZERO;
    for k in (1..b.len()).rev() {
        tail += b[k] - mul(a[k], gain);
        z[k - 1] = tail;
    }
    z.iter().all(finite).then_some(z)
}
/// Binary exponent of the largest part of `v` (0 when all parts are zero).
fn exponent_of(v: &[C]) -> i32 {
    v.iter()
        .flat_map(|z| [z.re, z.im])
        .filter(|&x| x != 0.)
        .map(libm::ilogb)
        .max()
        .unwrap_or(0)
}
/// `v` times 2^e (exact unless a part underflows).
fn scaled(v: &[C], e: i32) -> Vec<C> {
    v.iter()
        .map(|z| C::new(libm::scalbn(z.re, e), libm::scalbn(z.im, e)))
        .collect()
}
