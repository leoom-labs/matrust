//! fzero, fminsearch and fminbnd.
//!
//! fzero is Brent's zero finder (R. P. Brent, Algorithms for Minimization without
//! Derivatives, 1973, ch. 4; Forsythe, Malcolm & Moler, Computer Methods for
//! Mathematical Computations, 1977, sec. 7.2 "zeroin"): secant and inverse quadratic
//! interpolation steps, safeguarded by bisection. From a scalar start it first widens
//! a symmetric search until the sign changes (see `polyopt_fzero`).
//! fminbnd is Brent's one-dimensional minimiser (Brent 1973, ch. 5; Forsythe, Malcolm
//! & Moler 1977, sec. 8.2 "fmin"): golden-section search with parabolic interpolation.
//! fminsearch is the Nelder-Mead simplex method (Nelder & Mead, "A simplex method for
//! function minimization", Computer J. 7, 1965) in the form analysed by Lagarias,
//! Reeds, Wright & Wright ("Convergence properties of the Nelder-Mead simplex method
//! in low dimensions", SIAM J. Optim. 9, 1998): reflection 1, expansion 2, contraction
//! 1/2, shrink 1/2, with their ordering and acceptance rules.
use super::{BUDGET, Engine, Execution, Value, matrix_value, real_matrix, scalar_of};
use crate::engine::text_format::{mat2str_complex, mat2str_real};
use crate::{Complex64, Matrix, Result};
use std::cmp::Ordering;

/// fzero's TolX: Brent's absolute tolerance t, so steps stop at 2 eps |x| + eps/2.
const FZERO_TOL_X: f64 = f64::EPSILON;
/// Most halvings towards the far bracket end that fzero tries after a NaN value.
const FZERO_NAN_PROBES: usize = 60;
/// fminbnd's TolX and iteration limit.
const FMINBND_TOL_X: f64 = 1e-4;
const FMINBND_MAX_ITER: usize = 500;
/// fminsearch's TolX and TolFun; both must hold to stop.
const FMINSEARCH_TOL: f64 = 1e-4;

fn ascending(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}
/// a < b with NaN above every number (the simplex order of `ascending`).
fn below(a: f64, b: f64) -> bool {
    ascending(a, b) == Ordering::Less
}

impl Engine {
    // TODO: accept function handles once they exist on main; today the function
    // is a name (user function file, local function or built-in) given as text.
    fn polyopt_function_name(&self, who: &str, value: &Value) -> Result<String> {
        match value {
            Value::Text(name) | Value::QuotedString(name) if !name.is_empty() => Ok(name.clone()),
            _ => Err(format!(
                "{who}: the function must be given by name as text, e.g. {who}('myfun', ...); function handles are not supported yet"
            )),
        }
    }
    /// One function value as a complex scalar (real results have im == 0).
    fn polyopt_value(&mut self, name: &str, x: Matrix, out: &mut Execution) -> Result<Complex64> {
        self.check_cancel()?;
        let value = self.call(name, vec![Value::Matrix(x)], out)?;
        match value {
            Value::Matrix(m) if m.len() == 1 => Ok(Complex64::new(m.data()[0], 0.)),
            Value::Logical(m) if m.len() == 1 => Ok(Complex64::new(f64::from(m.data()[0]), 0.)),
            Value::Complex(m) if m.len() == 1 => Ok(m.data()[0]),
            _ => Err(format!(
                "User function '{name}' must return a real scalar value"
            )),
        }
    }
    fn polyopt_f(&mut self, name: &str, x: Matrix, out: &mut Execution) -> Result<f64> {
        let v = self.polyopt_value(name, x, out)?;
        if v.im != 0. {
            return Err(format!(
                "User function '{name}' must return a real scalar value"
            ));
        }
        Ok(v.re)
    }
    fn polyopt_note_options(
        &mut self,
        who: &str,
        args: &[Value],
        first: usize,
        out: &mut Execution,
    ) -> Result<()> {
        if args.len() > first {
            self.emit(
                out,
                format!("Note: {who} ignores options and extra arguments in this version"),
            )?;
        }
        Ok(())
    }
    pub(super) fn polyopt_optimize(
        &mut self,
        name: &str,
        args: &[Value],
        nout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (min, max) = if name == "fminbnd" { (3, 4) } else { (2, 3) };
        if args.len() < min {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > max {
            return Err("Too many input arguments.".into());
        }
        let f = self.polyopt_function_name(name, &args[0])?;
        self.polyopt_note_options(name, args, min, out)?;
        let (x, fval, flag) = match name {
            "fzero" => self.polyopt_fzero(&f, &args[1], out)?,
            "fminsearch" => self.polyopt_fminsearch(&f, &args[1], out)?,
            _ => {
                let ax = scalar_of(&args[1], name, "the lower bound")?;
                let bx = scalar_of(&args[2], name, "the upper bound")?;
                if ax > bx {
                    // fminbnd: exitflag -2 and empty x and fval, with a notice.
                    self.emit(
                        out,
                        "Exiting due to infeasibility:  lower bound exceeds the corresponding upper bound."
                            .to_string(),
                    )?;
                    let empty = matrix_value(0, 0, Vec::new())?;
                    return Ok([empty.clone(), empty, matrix_value(1, 1, vec![-2.])?]
                        .into_iter()
                        .take(nout.max(1))
                        .collect());
                }
                self.polyopt_fminbnd(&f, ax, bx, out)?
            }
        };
        let mut outputs = vec![Value::Matrix(x)];
        if nout > 1 {
            outputs.push(matrix_value(1, 1, vec![fval])?);
        }
        if nout > 2 {
            outputs.push(matrix_value(1, 1, vec![flag])?);
        }
        Ok(outputs)
    }

    /// fzero(f, x0) on an interval [a b] (two-element x0) or from a starting point.
    ///
    /// From a scalar x0 the search widens symmetrically: with dx = |x0|/50 (1/50 when
    /// that is subnormal, e.g. at x0 = 0) it evaluates x0 - dx, then x0 + dx,
    /// multiplies dx by sqrt(2) and repeats. Every earlier probe has the sign of f(x0), so the first probe of the
    /// other sign and the previous point on its side bound the nearest sign change on
    /// that side; the left probe of each step is tried first. A NaN, Inf or complex
    /// value, or a probe that overflows to +-Inf, ends the search without a root.
    fn polyopt_fzero(
        &mut self,
        f: &str,
        x0: &Value,
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64)> {
        let start = real_matrix(x0, "fzero")?;
        let (a, fa, b, fb) = match *start.data() {
            [x] => {
                if !x.is_finite() {
                    return Err("fzero: the starting point must be finite".into());
                }
                let fx = self.polyopt_value(f, Matrix::scalar(x), out)?;
                if fx.im != 0. || !fx.re.is_finite() {
                    return Err(
                        "fzero: the function value at the starting point must be finite and real"
                            .into(),
                    );
                }
                let fx = fx.re;
                if fx == 0. {
                    return Ok((Matrix::scalar(x), fx, 1.));
                }
                // Nearest point on each side whose value has the sign of f(x0).
                let mut near = [(x, fx), (x, fx)];
                // A subnormal dx would stay put (or underflow to 0) under the sqrt(2)
                // growth, so such a start uses the step of x0 = 0.
                let mut dx = x.abs() / 50.;
                if dx < f64::MIN_POSITIVE {
                    dx = 1. / 50.;
                }
                // dx grows by sqrt(2) per round, so some probe overflows to +-Inf (flag
                // -6) after about 2200 rounds at the latest.
                'search: loop {
                    for (side, probe) in [x - dx, x + dx].into_iter().enumerate() {
                        let (last, f_last) = near[side];
                        if !probe.is_finite() {
                            return self.polyopt_fzero_abort(
                                probe,
                                Complex64::new(f_last, 0.),
                                out,
                            );
                        }
                        let fp = self.polyopt_value(f, Matrix::scalar(probe), out)?;
                        if fp.im != 0. || !fp.re.is_finite() {
                            return self.polyopt_fzero_abort(probe, fp, out);
                        }
                        if fp.re == 0. {
                            return Ok((Matrix::scalar(probe), 0., 1.));
                        }
                        if (fp.re > 0.) != (fx > 0.) {
                            break 'search (last, f_last, probe, fp.re);
                        }
                        near[side] = (probe, fp.re);
                    }
                    dx *= std::f64::consts::SQRT_2;
                }
            }
            [a, b] => {
                if !a.is_finite() || !b.is_finite() {
                    return Err("fzero: the interval endpoints must be finite".into());
                }
                let fa = self.polyopt_value(f, Matrix::scalar(a), out)?;
                let fb = self.polyopt_value(f, Matrix::scalar(b), out)?;
                if [fa, fb].iter().any(|v| v.im != 0. || !v.re.is_finite()) {
                    return Err(
                        "fzero: the function values at the interval endpoints must be finite and real"
                            .into(),
                    );
                }
                let (fa, fb) = (fa.re, fb.re);
                if fa == 0. {
                    return Ok((Matrix::scalar(a), fa, 1.));
                }
                if fb == 0. {
                    return Ok((Matrix::scalar(b), fb, 1.));
                }
                if (fa > 0.) == (fb > 0.) {
                    return Err(
                        "fzero: the function values at the interval endpoints must differ in sign"
                            .into(),
                    );
                }
                (a, fa, b, fb)
            }
            _ => {
                return Err(
                    "fzero: x0 must be a starting point or a two-element interval [a b]".into(),
                );
            }
        };
        let Some((x, fx)) = self.polyopt_fzero_brent(f, a, fa, b, fb, out)? else {
            return Ok((Matrix::scalar(f64::NAN), f64::NAN, -3.));
        };
        // At a pole the bracket closes on a sign change where |f| only grows.
        if fx.abs() > fa.abs().max(fb.abs()) {
            self.emit(
                out,
                "fzero: x may be near a singular point: the interval around the sign change"
                    .to_string(),
            )?;
            self.emit(
                out,
                format!(
                    "    shrank to the tolerance, but |f(x)| grew from {} to {} as it shrank.",
                    mat2str_real(fa.abs().max(fb.abs()), 6),
                    mat2str_real(fx.abs(), 6)
                ),
            )?;
            return Ok((Matrix::scalar(x), fx, -5.));
        }
        Ok((Matrix::scalar(x), fx, 1.))
    }
    /// Brent's zero finder on [a, b] with f(a), f(b) of opposite signs (Brent 1973,
    /// sec. 4.6 procedure zero; Forsythe, Malcolm & Moler 1977, sec. 7.2). b is the
    /// best point, c the other end of the bracket and a the previous b. A step uses
    /// inverse quadratic interpolation (the secant when a = c) only when it stays well
    /// inside the bracket and is less than half the step before last; otherwise it
    /// bisects. Stops when half the bracket is within 2 eps |b| + TolX/2 or f(b) = 0.
    ///
    /// A NaN value has no sign, so it never replaces a bracket end. The trial point is
    /// moved halfway towards c, again and again, until the function is finite there
    /// (an isolated NaN costs one extra evaluation); that point then takes the place
    /// of the trial and the next step bisects. If NaN persists up to c, the search
    /// ends with a message and Ok(None) (flag -3).
    fn polyopt_fzero_brent(
        &mut self,
        f: &str,
        mut a: f64,
        mut fa: f64,
        mut b: f64,
        mut fb: f64,
        out: &mut Execution,
    ) -> Result<Option<(f64, f64)>> {
        let (mut c, mut fc) = (a, fa);
        let mut d = b - a;
        let mut e = d;
        loop {
            if fc.abs() < fb.abs() {
                (a, fa) = (b, fb);
                (b, fb) = (c, fc);
                (c, fc) = (a, fa);
            }
            let tol = 2. * f64::EPSILON * b.abs() + 0.5 * FZERO_TOL_X;
            // Half the bracket; halving each end first cannot overflow.
            let m = 0.5 * c - 0.5 * b;
            if m.abs() <= tol || fb == 0. {
                return Ok(Some((b, fb)));
            }
            if e.abs() < tol || fa.abs() <= fb.abs() {
                d = m;
                e = m;
            } else {
                let s = fb / fa;
                let (mut p, mut q) = if a == c {
                    (2. * m * s, 1. - s)
                } else {
                    let (q, r) = (fa / fc, fb / fc);
                    (
                        s * (2. * m * q * (q - r) - (b - a) * (r - 1.)),
                        (q - 1.) * (r - 1.) * (s - 1.),
                    )
                };
                if p > 0. {
                    q = -q;
                } else {
                    p = -p;
                }
                let before_last = e;
                e = d;
                // A NaN p or q fails both tests and bisects.
                if 2. * p < 3. * m * q - (tol * q).abs() && p < (0.5 * before_last * q).abs() {
                    d = p / q;
                } else {
                    d = m;
                    e = m;
                }
            }
            (a, fa) = (b, fb);
            b += if d.abs() > tol {
                d
            } else if m > 0. {
                tol
            } else {
                -tol
            };
            let mut fnew = self.polyopt_f(f, Matrix::scalar(b), out)?;
            let mut probes = 0;
            while fnew.is_nan() {
                let next = b + 0.5 * (c - b);
                probes += 1;
                if probes > FZERO_NAN_PROBES || next == b || next == c {
                    self.emit(
                        out,
                        "Exiting fzero: the function returned NaN inside the interval".to_string(),
                    )?;
                    self.emit(
                        out,
                        format!(
                            "    around x = {} and no finite value was found next to it.",
                            mat2str_real(b, 6)
                        ),
                    )?;
                    return Ok(None);
                }
                b = next;
                fnew = self.polyopt_f(f, Matrix::scalar(b), out)?;
            }
            fb = fnew;
            // b and c bracketed the sign change before this step, and a is that b.
            if (fb > 0.) != (fa > 0.) {
                (c, fc) = (a, fa);
                d = b - a;
                e = d;
            } else if probes > 0 {
                d = c - b;
                e = d;
            }
        }
    }
    /// Report a failed sign-change search and return NaN with its exit flag: -4 for a
    /// complex value, -3 for NaN or Inf, -6 when the probe x itself ran off to +-Inf.
    fn polyopt_fzero_abort(
        &mut self,
        x: f64,
        fx: Complex64,
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64)> {
        let (flag, reason) = if fx.im != 0. {
            (-4., "complex function value encountered")
        } else if !fx.re.is_finite() {
            (-3., "NaN or Inf function value encountered")
        } else {
            (-6., "no sign change was found before x reached")
        };
        self.emit(
            out,
            "Exiting fzero: aborting search for an interval containing a sign change".to_string(),
        )?;
        if flag == -6. {
            self.emit(out, format!("    because {reason} {}.", mat2str_real(x, 6)))?;
            self.emit(out, "The function may have no real root.".to_string())?;
        } else {
            let value = if fx.im != 0. {
                mat2str_complex(fx.re, fx.im, 6)
            } else {
                mat2str_real(fx.re, 6)
            };
            self.emit(
                out,
                format!(
                    "    because {reason} at x = {}: f(x) = {value}.",
                    mat2str_real(x, 6)
                ),
            )?;
            self.emit(
                out,
                "Check the function, or start from another point or interval.".to_string(),
            )?;
        }
        Ok((Matrix::scalar(f64::NAN), f64::NAN, flag))
    }

    /// Nelder-Mead on the vertices of a simplex (Lagarias et al. 1998, sec. 2). The
    /// papers leave the first simplex open; it is x0 and, for each element j, x0 with
    /// element j raised by 5 % (set to 0.00025 when it is 0), the step MathWorks
    /// documents for fminsearch. Vertices are kept ordered by value with a stable sort,
    /// so a new vertex ranks after older ones of equal value and a shrink keeps the
    /// best vertex first (the paper's tie rules). Stops when every vertex is within
    /// TolX of the best in each element and within TolFun of it in value, or after
    /// 200 * numel(x0) steps or function evaluations (checked between steps).
    fn polyopt_fminsearch(
        &mut self,
        f: &str,
        x0: &Value,
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64)> {
        let start = real_matrix(x0, "fminsearch")?;
        if start.is_empty() {
            return Err("fminsearch: the starting point x0 must not be empty".into());
        }
        let (rows, cols, n) = (start.rows(), start.cols(), start.len());
        if (n + 1).saturating_mul(n).saturating_mul(8) > BUDGET {
            return Err(format!(
                "fminsearch: x0 has {n} elements; the simplex of {} points would exceed the memory budget",
                n + 1
            ));
        }
        let limit = 200 * n;
        let mut simplex: Vec<(f64, Vec<f64>)> = Vec::with_capacity(n + 1);
        for j in 0..=n {
            let mut v = start.data().to_vec();
            if j > 0 {
                let k = j - 1;
                v[k] = if v[k] == 0. { 0.00025 } else { 1.05 * v[k] };
            }
            let fv = self.polyopt_f(f, Matrix::from_column_slice(rows, cols, &v)?, out)?;
            simplex.push((fv, v));
        }
        let mut evaluations = n + 1;
        let mut steps = 0;
        let flag = loop {
            simplex.sort_by(|p, q| ascending(p.0, q.0));
            let (f_best, best) = (simplex[0].0, &simplex[0].1);
            let converged = simplex[1..].iter().all(|(fv, v)| {
                (fv - f_best).abs() <= FMINSEARCH_TOL
                    && v.iter()
                        .zip(best)
                        .all(|(x, y)| (x - y).abs() <= FMINSEARCH_TOL)
            });
            if converged {
                break 1.;
            }
            if steps >= limit || evaluations >= limit {
                self.emit(
                    out,
                    format!(
                        "fminsearch stopped after {steps} steps and {evaluations} function evaluations (limit {limit})."
                    ),
                )?;
                self.emit(
                    out,
                    format!(
                        "    The best function value so far is {}; exitflag 0.",
                        mat2str_real(f_best, 6)
                    ),
                )?;
                break 0.;
            }
            // Centroid of the n best vertices, then points on the line from the
            // worst vertex w through it: x(t) = (1 + t) m - t w.
            let mut m = vec![0.; n];
            for (_, v) in &simplex[..n] {
                for (s, x) in m.iter_mut().zip(v) {
                    *s += x;
                }
            }
            for s in &mut m {
                *s /= n as f64;
            }
            let (f_worst, worst) = (simplex[n].0, simplex[n].1.clone());
            let f_second_worst = simplex[n - 1].0;
            let line = |t: f64| -> Vec<f64> {
                m.iter()
                    .zip(&worst)
                    .map(|(c, w)| (1. + t) * c - t * w)
                    .collect()
            };
            let reflected = line(1.);
            let f_reflected =
                self.polyopt_f(f, Matrix::from_column_slice(rows, cols, &reflected)?, out)?;
            evaluations += 1;
            let mut accepted = None;
            if below(f_reflected, f_best) {
                let expanded = line(2.);
                let f_expanded =
                    self.polyopt_f(f, Matrix::from_column_slice(rows, cols, &expanded)?, out)?;
                evaluations += 1;
                accepted = Some(if below(f_expanded, f_reflected) {
                    (f_expanded, expanded)
                } else {
                    (f_reflected, reflected)
                });
            } else if below(f_reflected, f_second_worst) {
                accepted = Some((f_reflected, reflected));
            } else if below(f_reflected, f_worst) {
                // Outside contraction, accepted when no worse than the reflection.
                let contracted = line(0.5);
                let f_contracted =
                    self.polyopt_f(f, Matrix::from_column_slice(rows, cols, &contracted)?, out)?;
                evaluations += 1;
                if !below(f_reflected, f_contracted) {
                    accepted = Some((f_contracted, contracted));
                }
            } else {
                // Inside contraction, accepted when better than the worst vertex.
                let contracted = line(-0.5);
                let f_contracted =
                    self.polyopt_f(f, Matrix::from_column_slice(rows, cols, &contracted)?, out)?;
                evaluations += 1;
                if below(f_contracted, f_worst) {
                    accepted = Some((f_contracted, contracted));
                }
            }
            match accepted {
                Some(vertex) => simplex[n] = vertex,
                None => {
                    // Shrink every vertex halfway towards the best one.
                    let best = simplex[0].1.clone();
                    for (fv, v) in &mut simplex[1..] {
                        for (x, y) in v.iter_mut().zip(&best) {
                            *x = y + 0.5 * (*x - y);
                        }
                        *fv = self.polyopt_f(f, Matrix::from_column_slice(rows, cols, v)?, out)?;
                    }
                    evaluations += n;
                }
            }
            steps += 1;
        };
        let (fval, x) = simplex.swap_remove(0);
        Ok((Matrix::from_column_vec(rows, cols, x)?, fval, flag))
    }

    /// Brent's minimiser on [ax, bx] (Brent 1973, ch. 5 procedure localmin;
    /// Forsythe, Malcolm & Moler 1977, sec. 8.2). x is the best point, w the second
    /// best and v the previous w. A parabola through x, w and v gives the step when
    /// its minimum lies inside [a, b] and the step is under half the step before
    /// last; otherwise the step is golden section into the larger part of [a, b].
    /// Points closer than tol = sqrt(eps) |x| + TolX/3 to x or to the ends are not
    /// evaluated; stops when [a, b] is within 2 tol of x.
    fn polyopt_fminbnd(
        &mut self,
        f: &str,
        ax: f64,
        bx: f64,
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64)> {
        if !(bx - ax).is_finite() {
            return Err("fminbnd: the bounds must be finite".into());
        }
        // (3 - sqrt(5)) / 2, the golden-section fraction.
        let golden = 0.5 * (3. - 5f64.sqrt());
        let root_eps = f64::EPSILON.sqrt();
        let (mut a, mut b) = (ax, bx);
        let mut x = a + golden * (b - a);
        let mut fx = self.polyopt_f(f, Matrix::scalar(x), out)?;
        let (mut w, mut fw, mut v, mut fv) = (x, fx, x, fx);
        // d: the last step; e: the step before it.
        let (mut d, mut e) = (0f64, 0f64);
        let mut iterations = 0;
        let flag = loop {
            let mid = 0.5 * (a + b);
            let tol = root_eps * x.abs() + FMINBND_TOL_X / 3.;
            let tol2 = 2. * tol;
            if (x - mid).abs() <= tol2 - 0.5 * (b - a) {
                break 1.;
            }
            if iterations >= FMINBND_MAX_ITER {
                self.emit(
                    out,
                    format!(
                        "fminbnd stopped after {FMINBND_MAX_ITER} iterations; the best function value so far is {}; exitflag 0.",
                        mat2str_real(fx, 6)
                    ),
                )?;
                break 0.;
            }
            let mut parabolic = false;
            if e.abs() > tol {
                let r = (x - w) * (fx - fv);
                let mut q = (x - v) * (fx - fw);
                let mut p = (x - v) * q - (x - w) * r;
                q = 2. * (q - r);
                if q > 0. {
                    p = -p;
                } else {
                    q = -q;
                }
                let before_last = e;
                e = d;
                if p.abs() < (0.5 * q * before_last).abs() && p > q * (a - x) && p < q * (b - x) {
                    d = p / q;
                    let u = x + d;
                    // Never evaluate within tol2 of either end.
                    if u - a < tol2 || b - u < tol2 {
                        d = if x < mid { tol } else { -tol };
                    }
                    parabolic = true;
                }
            }
            if !parabolic {
                e = if x < mid { b - x } else { a - x };
                d = golden * e;
            }
            let u = x + if d.abs() >= tol {
                d
            } else if d > 0. {
                tol
            } else {
                -tol
            };
            let fu = self.polyopt_f(f, Matrix::scalar(u), out)?;
            iterations += 1;
            if fu <= fx {
                if u < x {
                    b = x;
                } else {
                    a = x;
                }
                (v, fv) = (w, fw);
                (w, fw) = (x, fx);
                (x, fx) = (u, fu);
            } else {
                if u < x {
                    a = u;
                } else {
                    b = u;
                }
                if fu <= fw || w == x {
                    (v, fv) = (w, fw);
                    (w, fw) = (u, fu);
                } else if fu <= fv || v == x || v == w {
                    (v, fv) = (u, fu);
                }
            }
        };
        Ok((Matrix::scalar(x), fx, flag))
    }
}
