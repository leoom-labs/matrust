//! Lobatto collocation for two-point boundary value problems.
//! Kierzenka & Shampine, ACM TOMS 27 (2001), residual control and mesh selection;
//! Kierzenka & Shampine, JNAIAM 3 (2008), sections 2--4, four-point formula,
//! continuous extension and scaled residual control. Newton linearization follows
//! Ascher, Mattheij & Russell (1995), chapters 5 and 9. Engine independent.
#![allow(clippy::needless_range_loop)]
use super::num::{Lu, R, numjac};

pub trait BvpModel {
    /// f(x, y, p).
    fn f(&mut self, x: f64, y: &[f64], p: &[f64]) -> R<Vec<f64>>;
    /// Residual of the boundary conditions bc(ya, yb, p).
    fn bc(&mut self, ya: &[f64], yb: &[f64], p: &[f64]) -> R<Vec<f64>>;
    /// Analytic df/dy (column-major) when FJacobian is given.
    fn fjac(&mut self, x: f64, y: &[f64], p: &[f64]) -> R<Option<Vec<f64>>>;
    fn check_cancel(&self) -> R<()>;
}

#[derive(Clone, Copy, PartialEq)]
pub enum Method {
    /// bvp4c.
    Simpson,
    /// bvp5c.
    Lobatto4,
}

pub struct BvpOpts {
    pub rtol: f64,
    pub atol: Vec<f64>,
    pub nmax: usize,
}

pub struct BvpOut {
    pub x: Vec<f64>,
    pub y: Vec<Vec<f64>>,
    pub yp: Vec<Vec<f64>>,
    /// bvp5c interior stage values (two per interval).
    pub stages: Vec<[Vec<f64>; 2]>,
    pub p: Vec<f64>,
    pub maxres: f64,
    pub node_evals: usize,
    pub bc_evals: usize,
    pub warnings: Vec<String>,
}

/// Largest collocation system solved (unknowns), as the dense LU bound.
pub const MAX_UNKNOWNS: usize = 3000;
const SQ5: f64 = 2.236_067_977_499_79;

fn lobatto4() -> ([f64; 4], [[f64; 4]; 4], [f64; 4]) {
    let c = [0., (5. - SQ5) / 10., (5. + SQ5) / 10., 1.];
    let a = std::array::from_fn(|i| std::array::from_fn(|j| basis_integral(&c, j, c[i])));
    (c, a, [1. / 12., 5. / 12., 5. / 12., 1. / 12.])
}

/// Integral from zero to s of a cardinal Lagrange polynomial.
/// Integrating the slope interpolant gives the collocation tableau (2008, §2).
pub fn basis_integral(c: &[f64; 4], l: usize, s: f64) -> f64 {
    let mut coeff = [1., 0., 0., 0.];
    let mut degree = 0;
    let mut denominator = 1.;
    for j in 0..4 {
        if j != l {
            for k in (1..=degree + 1).rev() {
                coeff[k] = coeff[k - 1] - c[j] * coeff[k];
            }
            coeff[0] *= -c[j];
            denominator *= c[l] - c[j];
            degree += 1;
        }
    }
    s * (coeff[0] + s * (coeff[1] / 2. + s * (coeff[2] / 3. + s * coeff[3] / 4.))) / denominator
}

/// Endpoint value/slope interpolation; de Boor, A Practical Guide to Splines
/// (2001), chapter IV. Also used by the delay-solution evaluator.
pub fn hermite(
    x: f64,
    x0: f64,
    y0: &[f64],
    yp0: &[f64],
    x1: f64,
    y1: &[f64],
    yp1: &[f64],
) -> (Vec<f64>, Vec<f64>) {
    let h = x1 - x0;
    let s = (x - x0) / h;
    let mut y = Vec::with_capacity(y0.len());
    let mut yp = Vec::with_capacity(y0.len());
    for k in 0..y0.len() {
        let delta = (y1[k] - y0[k]) / h;
        let b = 3. * delta - 2. * yp0[k] - yp1[k];
        let a = yp0[k] + yp1[k] - 2. * delta;
        y.push(y0[k] + h * s * (yp0[k] + s * (b + s * a)));
        yp.push(yp0[k] + s * (2. * b + 3. * s * a));
    }
    (y, yp)
}

pub fn lobatto_eval(x: f64, x0: f64, h: f64, y0: &[f64], f: [&[f64]; 4]) -> (Vec<f64>, Vec<f64>) {
    let (c, _, _) = lobatto4();
    let s = (x - x0) / h;
    let mut y = y0.to_vec();
    let mut yp = vec![0.; y.len()];
    for l in 0..4 {
        let integral = basis_integral(&c, l, s);
        let mut basis = 1.;
        for j in 0..4 {
            if j != l {
                basis *= (s - c[j]) / (c[l] - c[j]);
            }
        }
        for k in 0..y.len() {
            y[k] += h * integral * f[l][k];
            yp[k] += basis * f[l][k];
        }
    }
    (y, yp)
}

struct Ctx<'a> {
    m: &'a mut dyn BvpModel,
    neq: usize,
    npar: usize,
    atol: Vec<f64>,
    node_evals: usize,
    bc_evals: usize,
}

/// Only numerical non-finiteness is recoverable during a trial step. Preserve
/// callback failures, cancellation and shape errors without matching text.
#[derive(Debug)]
enum EvalError {
    NonFinite(&'static str),
    Fatal(String),
}
impl From<String> for EvalError {
    fn from(message: String) -> Self {
        Self::Fatal(message)
    }
}
impl From<EvalError> for String {
    fn from(error: EvalError) -> Self {
        match error {
            EvalError::NonFinite(message) => message.into(),
            EvalError::Fatal(message) => message,
        }
    }
}
type EvalResult<T> = Result<T, EvalError>;

impl Ctx<'_> {
    fn f(&mut self, x: f64, y: &[f64], p: &[f64]) -> EvalResult<Vec<f64>> {
        self.m.check_cancel()?;
        self.node_evals += 1;
        let v = self.m.f(x, y, p)?;
        const MESSAGE: &str = "BVP derivative must be a finite vector of the expected length.";
        if v.len() != self.neq {
            return Err(EvalError::Fatal(MESSAGE.into()));
        }
        if v.iter().any(|v| !v.is_finite()) {
            return Err(EvalError::NonFinite(MESSAGE));
        }
        Ok(v)
    }
    fn bc(&mut self, ya: &[f64], yb: &[f64], p: &[f64]) -> EvalResult<Vec<f64>> {
        self.m.check_cancel()?;
        self.bc_evals += 1;
        let v = self.m.bc(ya, yb, p)?;
        const MESSAGE: &str =
            "BVP boundary residual must be a finite vector of the expected length.";
        if v.len() != self.neq + self.npar {
            return Err(EvalError::Fatal(MESSAGE.into()));
        }
        if v.iter().any(|v| !v.is_finite()) {
            return Err(EvalError::NonFinite(MESSAGE));
        }
        Ok(v)
    }
    fn jac(&mut self, x: f64, y: &[f64], p: &[f64], fv: &[f64]) -> R<(Vec<f64>, Vec<f64>)> {
        self.m.check_cancel()?;
        let j = match self.m.fjac(x, y, p)? {
            Some(j) => {
                if j.len() != self.neq * self.neq || j.iter().any(|v| !v.is_finite()) {
                    return Err("BVP FJacobian must be a finite square matrix.".into());
                }
                j
            }
            None => {
                let threshold = self.atol.clone();
                numjac(
                    &mut |v| self.f(x, v, p).map_err(String::from),
                    y,
                    fv,
                    &threshold,
                    &mut Vec::new(),
                )?
                .0
            }
        };
        let jp = if p.is_empty() {
            Vec::new()
        } else {
            numjac(
                &mut |v| self.f(x, y, v).map_err(String::from),
                p,
                fv,
                &[1.],
                &mut Vec::new(),
            )?
            .0
        };
        Ok((j, jp))
    }
}

fn matmul(a: &[f64], b: &[f64], n: usize, cols: usize) -> Vec<f64> {
    matmul_rect(a, b, n, cols)
}

/// Node unknowns first, then the two interior stages of each four-point
/// interval, then global parameters. Equations are interval blocks then BCs.
struct System {
    x: Vec<f64>,
    method: Method,
}

impl System {
    fn unknowns(&self, neq: usize, npar: usize) -> usize {
        let nodes = self.x.len();
        let blocks = if self.method == Method::Simpson {
            nodes
        } else {
            nodes.saturating_add(2 * nodes.saturating_sub(1))
        };
        blocks.saturating_mul(neq).saturating_add(npar)
    }
    fn stage_offset(&self, neq: usize, i: usize, s: usize) -> usize {
        (self.x.len() + 2 * i + s) * neq
    }
    fn eval(
        &self,
        cx: &mut Ctx,
        xs: &[f64],
        jac: bool,
    ) -> EvalResult<(Vec<f64>, Option<Vec<f64>>)> {
        let n = cx.neq;
        let np = cx.npar;
        let total = self.unknowns(n, np);
        let po = total - np;
        let p = &xs[po..];
        let mut residual = vec![0.; total];
        let mut matrix = jac.then(|| vec![0.; total * total]);
        let mut f = Vec::with_capacity(self.x.len());
        let mut j = Vec::new();
        for i in 0..self.x.len() {
            f.push(cx.f(self.x[i], &xs[i * n..(i + 1) * n], p)?);
            if jac {
                j.push(cx.jac(self.x[i], &xs[i * n..(i + 1) * n], p, &f[i])?);
            }
        }
        let (c, a, _) = lobatto4();
        let rows_per_interval = if self.method == Method::Simpson {
            n
        } else {
            3 * n
        };
        for i in 0..self.x.len() - 1 {
            cx.m.check_cancel()?;
            let h = self.x[i + 1] - self.x[i];
            let ya = &xs[i * n..(i + 1) * n];
            let yb = &xs[(i + 1) * n..(i + 2) * n];
            let row = i * rows_per_interval;
            if self.method == Method::Simpson {
                let ym: Vec<_> = (0..n)
                    .map(|k| 0.5 * (ya[k] + yb[k]) + h / 8. * (f[i][k] - f[i + 1][k]))
                    .collect();
                let fm = cx.f(self.x[i] + 0.5 * h, &ym, p)?;
                for k in 0..n {
                    residual[row + k] =
                        yb[k] - ya[k] - h / 6. * (f[i][k] + 4. * fm[k] + f[i + 1][k]);
                }
                if let Some(matrix) = matrix.as_mut() {
                    let (jm, pm) = cx.jac(self.x[i] + 0.5 * h, &ym, p, &fm)?;
                    let mut da: Vec<_> = j[i].0.iter().map(|v| h / 8. * v).collect();
                    let mut db: Vec<_> = j[i + 1].0.iter().map(|v| -h / 8. * v).collect();
                    for k in 0..n {
                        da[k + k * n] += 0.5;
                        db[k + k * n] += 0.5;
                    }
                    let ma = matmul(&jm, &da, n, n);
                    let mb = matmul(&jm, &db, n, n);
                    for col in 0..n {
                        for k in 0..n {
                            let d = if k == col { 1. } else { 0. };
                            matrix[row + k + (i * n + col) * total] =
                                -d - h / 6. * (j[i].0[k + col * n] + 4. * ma[k + col * n]);
                            matrix[row + k + ((i + 1) * n + col) * total] =
                                d - h / 6. * (j[i + 1].0[k + col * n] + 4. * mb[k + col * n]);
                        }
                    }
                    let dp: Vec<_> = (0..n * np)
                        .map(|k| h / 8. * (j[i].1[k] - j[i + 1].1[k]))
                        .collect();
                    let mp = matmul_rect(&jm, &dp, n, np);
                    for col in 0..np {
                        for k in 0..n {
                            matrix[row + k + (po + col) * total] = -h / 6.
                                * (j[i].1[k + col * n]
                                    + j[i + 1].1[k + col * n]
                                    + 4. * (pm[k + col * n] + mp[k + col * n]));
                        }
                    }
                }
            } else {
                let o1 = self.stage_offset(n, i, 0);
                let o2 = self.stage_offset(n, i, 1);
                let f1 = cx.f(self.x[i] + c[1] * h, &xs[o1..o1 + n], p)?;
                let f2 = cx.f(self.x[i] + c[2] * h, &xs[o2..o2 + n], p)?;
                let fs = [&f[i], &f1, &f2, &f[i + 1]];
                let offsets = [i * n, o1, o2, (i + 1) * n];
                let js = if jac {
                    vec![
                        j[i].clone(),
                        cx.jac(self.x[i] + c[1] * h, &xs[o1..o1 + n], p, &f1)?,
                        cx.jac(self.x[i] + c[2] * h, &xs[o2..o2 + n], p, &f2)?,
                        j[i + 1].clone(),
                    ]
                } else {
                    Vec::new()
                };
                for stage in 1..4 {
                    for k in 0..n {
                        let r = row + (stage - 1) * n + k;
                        residual[r] = xs[offsets[stage] + k]
                            - ya[k]
                            - h * (0..4).map(|l| a[stage][l] * fs[l][k]).sum::<f64>();
                        if let Some(matrix) = matrix.as_mut() {
                            matrix[r + (i * n + k) * total] -= 1.;
                            matrix[r + (offsets[stage] + k) * total] += 1.;
                            for l in 0..4 {
                                for col in 0..n {
                                    matrix[r + (offsets[l] + col) * total] -=
                                        h * a[stage][l] * js[l].0[k + col * n];
                                }
                                for col in 0..np {
                                    matrix[r + (po + col) * total] -=
                                        h * a[stage][l] * js[l].1[k + col * n];
                                }
                            }
                        }
                    }
                }
            }
        }
        let last = (self.x.len() - 1) * n;
        let boundary = cx.bc(&xs[..n], &xs[last..last + n], p)?;
        let row = total - n - np;
        residual[row..].copy_from_slice(&boundary);
        if let Some(matrix) = matrix.as_mut() {
            let mut v = xs[..n].to_vec();
            v.extend_from_slice(&xs[last..last + n]);
            v.extend_from_slice(p);
            let jb = numjac(
                &mut |z| {
                    cx.bc(&z[..n], &z[n..2 * n], &z[2 * n..])
                        .map_err(String::from)
                },
                &v,
                &boundary,
                &[1.],
                &mut Vec::new(),
            )?
            .0;
            for col in 0..v.len() {
                let offset = if col < n {
                    col
                } else if col < 2 * n {
                    last + col - n
                } else {
                    po + col - 2 * n
                };
                for k in 0..n + np {
                    matrix[row + k + offset * total] = jb[k + col * (n + np)];
                }
            }
        }
        if residual.iter().any(|v| !v.is_finite())
            || matrix
                .as_ref()
                .is_some_and(|m| m.iter().any(|v| !v.is_finite()))
        {
            return Err(EvalError::NonFinite(
                "BVP collocation equations overflowed.",
            ));
        }
        Ok((residual, matrix))
    }
}

fn matmul_rect(a: &[f64], b: &[f64], n: usize, cols: usize) -> Vec<f64> {
    use nalgebra::DMatrixView;
    (DMatrixView::from_slice(a, n, n) * DMatrixView::from_slice(b, n, cols))
        .as_slice()
        .to_vec()
}

/// Damped Newton with a residual merit function (Ascher et al., ch. 9).
/// Solve linear systems; never form an inverse. Callbacks remain sequential
/// because the engine model is mutable and need not be Send or Sync.
fn newton(cx: &mut Ctx, sys: &System, xs: &mut Vec<f64>, rtol: f64) -> R<bool> {
    let initial = xs.clone();
    if damped_newton(cx, sys, xs, rtol)? {
        return Ok(true);
    }
    // A grossly inconsistent boundary guess can drive every Newton direction
    // towards the edge of the ODE domain or make its linear system unusable.
    // Try one boundary-corrected start,
    // retaining the shape of the original guess (including interior stages).
    // This is a bounded globalization safeguard, not an alternate ODE solver.
    if let Some(start) = boundary_corrected_start(cx, sys, &initial)? {
        *xs = start;
        return damped_newton(cx, sys, xs, rtol);
    }
    Ok(false)
}

fn boundary_corrected_start(cx: &mut Ctx, sys: &System, xs: &[f64]) -> R<Option<Vec<f64>>> {
    let n = cx.neq;
    let np = cx.npar;
    let po = xs.len() - np;
    let last = (sys.x.len() - 1) * n;
    let boundary = cx.bc(&xs[..n], &xs[last..last + n], &xs[po..])?;
    let norm = |v: &[f64]| v.iter().fold(0f64, |acc, x| acc.max(x.abs()));
    if norm(&boundary) == 0. {
        return Ok(None);
    }
    // Constant shifts of each state/parameter preserve the guess's shape.
    // Boundary conditions may not constrain all shifts: a minimum-norm SVD
    // solve handles rank deficiency without explicit inversion.
    let matrix = numjac(
        &mut |shift| {
            let a: Vec<_> = (0..n).map(|k| xs[k] + shift[k]).collect();
            let b: Vec<_> = (0..n).map(|k| xs[last + k] + shift[k]).collect();
            let p: Vec<_> = (0..np).map(|k| xs[po + k] + shift[n + k]).collect();
            cx.bc(&a, &b, &p).map_err(String::from)
        },
        &vec![0.; n + np],
        &boundary,
        &[1.],
        &mut Vec::new(),
    )?
    .0;
    if matrix.iter().any(|x| !x.is_finite()) {
        return Ok(None);
    }
    cx.m.check_cancel()?;
    let Some(svd) = nalgebra::DMatrix::from_column_slice(n + np, n + np, &matrix).try_svd(
        true,
        true,
        f64::EPSILON,
        1000,
    ) else {
        return Ok(None);
    };
    let cutoff = f64::EPSILON * (n + np) as f64 * svd.singular_values.max();
    let Ok(shift) = svd.solve(&nalgebra::DVector::from_column_slice(&boundary), cutoff) else {
        return Ok(None);
    };
    cx.m.check_cancel()?;
    if shift.iter().any(|x| !x.is_finite()) {
        return Ok(None);
    }
    let mut candidate = xs.to_vec();
    for (i, x) in candidate.iter_mut().enumerate() {
        *x -= shift[if i < po { i % n } else { n + i - po }];
    }
    let corrected = match cx.bc(
        &candidate[..n],
        &candidate[last..last + n],
        &candidate[po..],
    ) {
        Ok(value) => value,
        Err(EvalError::NonFinite(_)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if norm(&corrected) >= 0.5 * norm(&boundary) {
        return Ok(None);
    }
    match sys.eval(cx, &candidate, false) {
        Ok(_) => Ok(Some(candidate)),
        Err(EvalError::NonFinite(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn damped_newton(cx: &mut Ctx, sys: &System, xs: &mut Vec<f64>, rtol: f64) -> R<bool> {
    let tolerance = (rtol * 1e-3).max(100. * f64::EPSILON);
    // Interval equations represent an integrated defect. A fixed absolute
    // stopping test would accept inaccurate slopes on a refined mesh.
    let n = cx.neq;
    let np = cx.npar;
    let norm = |point: &[f64], r: &[f64], atol: &[f64]| {
        let blocks = if sys.method == Method::Simpson { 1 } else { 3 };
        let mut largest = 0f64;
        for i in 0..sys.x.len() - 1 {
            let h = (sys.x[i + 1] - sys.x[i]).abs();
            for block in 0..blocks {
                for k in 0..n {
                    let weight = point[i * n + k]
                        .abs()
                        .max(point[(i + 1) * n + k].abs())
                        .max(atol[k] / rtol);
                    let defect = r[(i * blocks + block) * n + k].abs();
                    let scaled = if sys.method == Method::Simpson {
                        // Subtraction of neighboring values has an O(eps*weight)
                        // floor before division by h. Use a per-interval floor,
                        // not a global relaxation caused by the smallest cell.
                        defect / h / weight / tolerance.max(32. * f64::EPSILON / h)
                    } else {
                        defect / weight / tolerance
                    };
                    largest = largest.max(scaled);
                }
            }
        }
        let offset = (sys.x.len() - 1) * blocks * n;
        for k in 0..n + np {
            largest = largest.max(r[offset + k].abs() / tolerance);
        }
        largest
    };
    for _ in 0..24 {
        cx.m.check_cancel()?;
        let (r, j) = sys.eval(cx, xs, true)?;
        let merit = norm(xs, &r, &cx.atol);
        if merit <= 1. {
            return Ok(true);
        }
        cx.m.check_cancel()?;
        let delta = Lu::new(&j.unwrap(), xs.len()).solve(&r);
        cx.m.check_cancel()?;
        if delta.iter().any(|v| !v.is_finite()) {
            return Ok(false);
        }
        let mut damping = 1.;
        let mut accepted = false;
        for _ in 0..14 {
            let trial: Vec<_> = xs
                .iter()
                .zip(&delta)
                .map(|(x, d)| x - damping * d)
                .collect();
            if trial.iter().all(|v| v.is_finite()) {
                match sys.eval(cx, &trial, false) {
                    Ok((rr, _)) => {
                        // Compare both residuals in the current iterate's norm
                        // (Ascher et al., ch. 9), even when a trial shrinks y.
                        if norm(xs, &rr, &cx.atol) < merit * (1. - 1e-4 * damping) {
                            *xs = trial;
                            accepted = true;
                            break;
                        }
                    }
                    Err(EvalError::NonFinite(_)) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            damping *= 0.5;
        }
        if !accepted {
            return Ok(false);
        }
    }
    Ok(false)
}

/// Continuous extension on an already solved interval.
fn interval_value(
    out: &BvpOut,
    method: Method,
    slopes: &[[Vec<f64>; 2]],
    i: usize,
    x: f64,
) -> (Vec<f64>, Vec<f64>) {
    if method == Method::Simpson {
        hermite(
            x,
            out.x[i],
            &out.y[i],
            &out.yp[i],
            out.x[i + 1],
            &out.y[i + 1],
            &out.yp[i + 1],
        )
    } else {
        lobatto_eval(
            x,
            out.x[i],
            out.x[i + 1] - out.x[i],
            &out.y[i],
            [&out.yp[i], &slopes[i][0], &slopes[i][1], &out.yp[i + 1]],
        )
    }
}

fn refinement_warning(limit: Option<usize>, nodes: usize, residual: f64, rtol: f64) -> String {
    let reason = match limit {
        Some(limit) => format!("without using more than {limit} mesh points"),
        None => "within 32 mesh solves".into(),
    };
    format!(
        "Warning: Unable to meet the tolerance {reason}. The last mesh and solution are returned.\nThe last mesh has {nodes} points; maximum residual is {residual:.3e}, requested tolerance is {rtol:.3e}."
    )
}

pub fn solve(
    m: &mut dyn BvpModel,
    method: Method,
    x0: &[f64],
    y0: &[Vec<f64>],
    p0: &[f64],
    o: &BvpOpts,
) -> R<BvpOut> {
    m.check_cancel()?;
    let n = y0.first().map_or(0, Vec::len);
    if n == 0
        || x0.len() < 2
        || y0.len() != x0.len()
        || y0
            .iter()
            .any(|v| v.len() != n || v.iter().any(|v| !v.is_finite()))
        || x0.iter().chain(p0).any(|v| !v.is_finite())
    {
        return Err(
            "BVP initial mesh, guess and parameters must be finite and have consistent dimensions."
                .into(),
        );
    }
    let direction = (x0[1] - x0[0]).signum();
    if x0
        .windows(2)
        .any(|w| (w[1] - w[0]) * direction <= 0. || !(w[1] - w[0]).is_finite())
    {
        return Err("BVP mesh must be strictly monotone.".into());
    }
    if !o.rtol.is_finite()
        || o.rtol <= 0.
        || (o.atol.len() != 1 && o.atol.len() != n)
        || o.atol.iter().any(|v| !v.is_finite() || *v <= 0.)
    {
        return Err(
            "BVP tolerances must be positive, finite and have compatible dimensions.".into(),
        );
    }
    let mut sys = System {
        x: x0.to_vec(),
        method,
    };
    if sys.unknowns(n, p0.len()) > MAX_UNKNOWNS {
        return Err(format!(
            "BVP collocation system exceeds the limit of {MAX_UNKNOWNS} unknowns."
        ));
    }
    let mut cx = Ctx {
        m,
        neq: n,
        npar: p0.len(),
        atol: (0..n)
            .map(|k| o.atol[if o.atol.len() == 1 { 0 } else { k }])
            .collect(),
        node_evals: 0,
        bc_evals: 0,
    };
    let mut nodes = y0.to_vec();
    let mut stages: Vec<[Vec<f64>; 2]> = nodes
        .windows(2)
        .filter(|_| method == Method::Lobatto4)
        .map(|w| {
            std::array::from_fn(|s| {
                w[0].iter()
                    .zip(&w[1])
                    .map(|(a, b)| a + (b - a) * lobatto4().0[s + 1])
                    .collect()
            })
        })
        .collect();
    let mut p = p0.to_vec();
    for refinement in 0..32 {
        let mut xs: Vec<_> = nodes.iter().flatten().copied().collect();
        if method == Method::Lobatto4 {
            xs.extend(stages.iter().flat_map(|s| s.iter().flatten()).copied());
        }
        xs.extend_from_slice(&p);
        if !newton(&mut cx, &sys, &mut xs, o.rtol)? {
            return Err(
                "Unable to solve the collocation equations -- a singular Jacobian encountered."
                    .into(),
            );
        }
        let po = xs.len() - p.len();
        p.copy_from_slice(&xs[po..]);
        nodes = xs[..sys.x.len() * n]
            .chunks_exact(n)
            .map(|v| v.to_vec())
            .collect();
        if method == Method::Lobatto4 {
            stages = (0..sys.x.len() - 1)
                .map(|i| {
                    std::array::from_fn(|s| {
                        xs[sys.stage_offset(n, i, s)..sys.stage_offset(n, i, s) + n].to_vec()
                    })
                })
                .collect();
        }
        let yp = nodes
            .iter()
            .zip(&sys.x)
            .map(|(y, x)| cx.f(*x, y, &p))
            .collect::<EvalResult<Vec<_>>>()?;
        let mut out = BvpOut {
            x: sys.x.clone(),
            y: nodes.clone(),
            yp,
            stages: if method == Method::Lobatto4 {
                stages.clone()
            } else {
                Vec::new()
            },
            p: p.clone(),
            maxres: 0.,
            node_evals: 0,
            bc_evals: 0,
            warnings: Vec::new(),
        };
        let mut slopes = Vec::new();
        if method == Method::Lobatto4 {
            let c = lobatto4().0;
            for i in 0..stages.len() {
                let h = sys.x[i + 1] - sys.x[i];
                slopes.push([
                    cx.f(sys.x[i] + c[1] * h, &stages[i][0], &p)?,
                    cx.f(sys.x[i] + c[2] * h, &stages[i][1], &p)?,
                ]);
            }
        }
        let mut errors = Vec::new();
        // Simpson: subinterval L2 norm of each scaled residual, using five-point
        // Lobatto quadrature (Kierzenka & Shampine 2001, residual control).
        // Endpoint residuals vanish by construction. Retain the midpoint term
        // to include the remaining nonlinear-solve defect. The integral is not
        // divided by h: this is an L2 norm, not an RMS residual.
        // Four-point collocation: keep the extra samples of the scaled defect
        // (Kierzenka & Shampine 2008, §3.2).
        for i in 0..sys.x.len() - 1 {
            let h = sys.x[i + 1] - sys.x[i];
            let mut error = 0f64;
            let mut l2 = if method == Method::Simpson {
                vec![0f64; n]
            } else {
                Vec::new()
            };
            let samples: &[f64] = if method == Method::Simpson {
                &[0.172_673_164_646_011_46, 0.5, 0.827_326_835_353_988_5]
            } else {
                &[
                    0.112_701_665_379_258_3,
                    (5. - SQ5) / 10.,
                    0.5,
                    (5. + SQ5) / 10.,
                    0.887_298_334_620_741_7,
                ]
            };
            for (sample, &s) in samples.iter().enumerate() {
                let x = sys.x[i] + s * h;
                let (v, d) = interval_value(&out, method, &slopes, i, x);
                if v.iter().chain(&d).any(|v| !v.is_finite()) {
                    return Err("BVP continuous extension overflowed.".into());
                }
                let f = cx.f(x, &v, &p)?;
                for k in 0..n {
                    let weight = if method == Method::Simpson {
                        f[k].abs()
                    } else {
                        v[k].abs().max(nodes[i][k].abs()).max(nodes[i + 1][k].abs())
                    };
                    let scale = weight.max(cx.atol[k] / o.rtol);
                    let defect = (d[k] - f[k]).abs() / scale;
                    if method == Method::Simpson {
                        let quadrature_weight: f64 =
                            if sample == 1 { 16. / 45. } else { 49. / 180. };
                        l2[k] = l2[k].hypot(quadrature_weight.sqrt() * defect);
                    } else {
                        error = error.max(h.abs() * defect);
                    }
                }
            }
            if method == Method::Simpson {
                error = h.abs().sqrt() * l2.into_iter().fold(0f64, f64::max);
            }
            if !error.is_finite() {
                return Err("BVP residual estimate overflowed.".into());
            }
            errors.push(error);
            out.maxres = out.maxres.max(error);
        }
        out.node_evals = cx.node_evals;
        out.bc_evals = cx.bc_evals;
        if out.maxres <= o.rtol {
            return Ok(out);
        }
        let splits: Vec<_> = errors
            .iter()
            .map(|e| {
                if *e <= o.rtol {
                    1
                } else {
                    ((*e / o.rtol).powf(if method == Method::Simpson {
                        1. / 3.
                    } else {
                        1. / 5.
                    }) * 1.1)
                        .ceil()
                        .clamp(2., 4.) as usize
                }
            })
            .collect();
        let count = 1 + splits.iter().sum::<usize>();
        let new_system = System {
            x: vec![0.; count],
            method,
        };
        if count > o.nmax || new_system.unknowns(n, p.len()) > MAX_UNKNOWNS {
            let limit = o.nmax.min(if method == Method::Simpson {
                (MAX_UNKNOWNS - p.len()) / n
            } else {
                ((MAX_UNKNOWNS - p.len()) / n).div_ceil(3)
            });
            out.warnings.push(refinement_warning(
                Some(limit),
                out.x.len(),
                out.maxres,
                o.rtol,
            ));
            return Ok(out);
        }
        if refinement == 31 {
            out.warnings
                .push(refinement_warning(None, out.x.len(), out.maxres, o.rtol));
            return Ok(out);
        }
        let mut new_x = vec![sys.x[0]];
        let mut new_y = vec![nodes[0].clone()];
        let mut new_stages = Vec::new();
        let c = lobatto4().0;
        for i in 0..splits.len() {
            for part in 0..splits[i] {
                let a = sys.x[i] + (sys.x[i + 1] - sys.x[i]) * (part as f64 / splits[i] as f64);
                let b = if part + 1 == splits[i] {
                    sys.x[i + 1]
                } else {
                    sys.x[i] + (sys.x[i + 1] - sys.x[i]) * ((part + 1) as f64 / splits[i] as f64)
                };
                if a == b {
                    return Err("BVP mesh spacing is too small for further refinement.".into());
                }
                if method == Method::Lobatto4 {
                    new_stages.push(std::array::from_fn(|s| {
                        interval_value(&out, method, &slopes, i, a + c[s + 1] * (b - a)).0
                    }));
                }
                new_x.push(b);
                new_y.push(if part + 1 == splits[i] {
                    nodes[i + 1].clone()
                } else {
                    interval_value(&out, method, &slopes, i, b).0
                });
            }
        }
        sys.x = new_x;
        nodes = new_y;
        stages = new_stages;
    }
    unreachable!("bounded mesh refinement returns on its final iteration")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Model {
        kind: usize,
        analytic: bool,
        cancelled: Cell<bool>,
        cancel_after: Option<usize>,
        calls: usize,
        nonfinite_calls: usize,
    }
    impl Model {
        fn new(kind: usize, analytic: bool) -> Self {
            Self {
                kind,
                analytic,
                cancelled: Cell::new(false),
                cancel_after: None,
                calls: 0,
                nonfinite_calls: 0,
            }
        }
    }
    impl BvpModel for Model {
        fn f(&mut self, x: f64, y: &[f64], p: &[f64]) -> R<Vec<f64>> {
            self.calls += 1;
            if self.cancel_after == Some(self.calls) {
                self.cancelled.set(true);
            }
            let value = match self.kind {
                0 => vec![y[1], -p.first().copied().unwrap_or(1.) * y[0]],
                1 => vec![y[1], 2. * y[0] * y[0] * y[0]], // y = 1/(2-x)
                2 => vec![y[1], 100. * y[0]],             // boundary layer exp(-10*x)
                3 => vec![2. * x],
                4 => vec![f64::NAN],
                6 => vec![y[0].sqrt()],
                7 => vec![y[1], libm::exp(y[0])],
                8 => vec![x * x * x],
                _ => vec![0.],
            };
            self.nonfinite_calls += usize::from(value.iter().any(|v| !v.is_finite()));
            Ok(value)
        }
        fn bc(&mut self, a: &[f64], b: &[f64], p: &[f64]) -> R<Vec<f64>> {
            Ok(match self.kind {
                0 if p.is_empty() => vec![a[0], b[0] - 1.],
                0 => vec![a[0], a[1] - 1., b[0]],
                1 => vec![a[0] - 0.5, b[0] - 1.],
                2 => vec![a[0] - 1., b[0] - libm::exp(-10.)],
                3 | 8 => vec![a[0]],
                6 => vec![a[0] - 1.],
                7 => vec![a[0], b[0]],
                _ => vec![1.],
            })
        }
        fn fjac(&mut self, _: f64, y: &[f64], p: &[f64]) -> R<Option<Vec<f64>>> {
            Ok(self.analytic.then(|| match self.kind {
                0 => vec![0., -p.first().copied().unwrap_or(1.), 1., 0.],
                1 => vec![0., 6. * y[0] * y[0], 1., 0.],
                2 => vec![0., 100., 1., 0.],
                _ => vec![0.],
            }))
        }
        fn check_cancel(&self) -> R<()> {
            if self.cancelled.get() {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        }
    }
    fn options(rtol: f64) -> BvpOpts {
        BvpOpts {
            rtol,
            atol: vec![rtol * 1e-3],
            nmax: 1000,
        }
    }
    fn evaluate(out: &BvpOut, method: Method, x: f64, model: &mut Model) -> (Vec<f64>, Vec<f64>) {
        let mut slopes = Vec::new();
        let c = lobatto4().0;
        if method == Method::Lobatto4 {
            for i in 0..out.x.len() - 1 {
                let h = out.x[i + 1] - out.x[i];
                slopes.push([
                    model
                        .f(out.x[i] + c[1] * h, &out.stages[i][0], &out.p)
                        .unwrap(),
                    model
                        .f(out.x[i] + c[2] * h, &out.stages[i][1], &out.p)
                        .unwrap(),
                ]);
            }
        }
        let i = out
            .x
            .partition_point(|v| *v <= x)
            .saturating_sub(1)
            .min(out.x.len() - 2);
        interval_value(out, method, &slopes, i, x)
    }
    #[test]
    fn cardinal_integrals_and_polynomial_extensions() {
        let (c, a, b) = lobatto4();
        for power in 0..4 {
            let f: Vec<Vec<_>> = c.iter().map(|v| vec![v.powi(power)]).collect();
            for s in [-0.4, 0., 0.13, 0.5, 1., 1.6] {
                let (y, d) = lobatto_eval(s, 0., 1., &[3.], [&f[0], &f[1], &f[2], &f[3]]);
                assert!((y[0] - 3. - s.powi(power + 1) / (power + 1) as f64).abs() < 2e-14);
                assert!((d[0] - s.powi(power)).abs() < 2e-14);
            }
            for i in 0..4 {
                let integral = (0..4).map(|j| a[i][j] * c[j].powi(power)).sum::<f64>();
                assert!((integral - c[i].powi(power + 1) / (power + 1) as f64).abs() < 2e-15);
            }
        }
        for j in 0..4 {
            assert!((a[3][j] - b[j]).abs() < 2e-15);
        }
        for (x0, x1) in [(-0.3, 1.2), (1.2, -0.3)] {
            let f = |x: f64| x * x * x - 2. * x * x + 3. * x - 1.;
            let d = |x: f64| 3. * x * x - 4. * x + 3.;
            for x in [-0.9, x0, 0.4, x1, 2.] {
                let (v, vp) = hermite(x, x0, &[f(x0)], &[d(x0)], x1, &[f(x1)], &[d(x1)]);
                assert!((v[0] - f(x)).abs() < 1e-13);
                assert!((vp[0] - d(x)).abs() < 1e-13);
            }
        }
    }
    #[test]
    fn assembled_jacobians_include_stages_parameters_and_boundary() {
        for method in [Method::Simpson, Method::Lobatto4] {
            let mut m = Model::new(0, true);
            let mut cx = Ctx {
                m: &mut m,
                neq: 2,
                npar: 1,
                atol: vec![1e-6; 2],
                node_evals: 0,
                bc_evals: 0,
            };
            let sys = System {
                x: vec![0., 0.3, 1.],
                method,
            };
            let xs: Vec<_> = (0..sys.unknowns(2, 1))
                .map(|i| 0.4 + i as f64 / 10.)
                .collect();
            let (_, j) = sys.eval(&mut cx, &xs, true).unwrap();
            let j = j.unwrap();
            let h = 1e-5;
            for col in 0..xs.len() {
                let mut xp = xs.clone();
                let mut xm = xs.clone();
                xp[col] += h;
                xm[col] -= h;
                let rp = sys.eval(&mut cx, &xp, false).unwrap().0;
                let rm = sys.eval(&mut cx, &xm, false).unwrap().0;
                for row in 0..xs.len() {
                    assert!(
                        (j[row + col * xs.len()] - (rp[row] - rm[row]) / (2. * h)).abs() < 2e-7,
                        "row={row} col={col}"
                    );
                }
            }
        }
    }
    #[test]
    fn oscillator_and_unknown_eigenvalue() {
        for method in [Method::Simpson, Method::Lobatto4] {
            for analytic in [false, true] {
                let mut m = Model::new(0, analytic);
                let x: Vec<_> = (0..5)
                    .map(|i| std::f64::consts::FRAC_PI_2 * i as f64 / 4.)
                    .collect();
                let out = solve(
                    &mut m,
                    method,
                    &x,
                    &vec![vec![0.; 2]; 5],
                    &[],
                    &options(1e-6),
                )
                .unwrap();
                assert!(
                    out.warnings.is_empty(),
                    "method={} analytic={analytic} mesh={} residual={} warnings={:?}",
                    if method == Method::Simpson { 4 } else { 5 },
                    out.x.len(),
                    out.maxres,
                    out.warnings
                );
                assert!(out.maxres <= 1e-6);
                for x in [0., 0.17, 0.5, 1., std::f64::consts::FRAC_PI_2] {
                    let (v, d) = evaluate(&out, method, x, &mut m);
                    assert!((v[0] - libm::sin(x)).abs() < 2e-7, "{x}: {v:?}");
                    assert!((v[1] - libm::cos(x)).abs() < 2e-7);
                    assert!((d[0] - v[1]).abs() < 3e-6);
                }
                let x: Vec<_> = (0..10)
                    .map(|i| std::f64::consts::PI * i as f64 / 9.)
                    .collect();
                let y: Vec<_> = x
                    .iter()
                    .map(|x| vec![libm::sin(*x), libm::cos(*x)])
                    .collect();
                let out = solve(&mut m, method, &x, &y, &[0.8], &options(1e-6)).unwrap();
                assert!((out.p[0] - 1.).abs() < 2e-7, "p={:?}", out.p);
                assert!(out.y[0][0].abs() < 1e-10);
                assert!((out.y[0][1] - 1.).abs() < 1e-10);
                assert!(out.y.last().unwrap()[0].abs() < 1e-10);
            }
        }
    }
    #[test]
    fn nonlinear_problem_and_boundary_layer() {
        for method in [Method::Simpson, Method::Lobatto4] {
            for kind in [1, 2] {
                let mut m = Model::new(kind, false);
                let x = vec![0., 0.25, 0.5, 0.75, 1.];
                // L2 residual control need not bound the pointwise error by
                // the same tolerance. Request more accuracy for this layer
                // while preserving the independent 1e-6 solution check.
                let opts = options(if method == Method::Simpson && kind == 2 {
                    1e-6
                } else {
                    1e-5
                });
                let out = solve(&mut m, method, &x, &vec![vec![0.5, 0.]; 5], &[], &opts).unwrap();
                assert!(out.warnings.is_empty(), "{:?}", out.warnings);
                assert!(out.maxres <= opts.rtol);
                for i in 0..101 {
                    let x = i as f64 / 100.;
                    let (v, _) = evaluate(&out, method, x, &mut m);
                    let want = if kind == 1 {
                        1. / (2. - x)
                    } else {
                        libm::exp(-10. * x)
                    };
                    assert!((v[0] - want).abs() < 1e-6, "kind={kind} x={x} {v:?}");
                }
            }
        }
    }
    #[test]
    fn newton_backtracks_outside_derivative_domain() {
        let mut m = Model::new(6, false);
        let out = solve(
            &mut m,
            Method::Lobatto4,
            &[0., 0.25, 0.5, 0.75, 1.],
            &vec![vec![1e-4]; 5],
            &[],
            &options(1e-3),
        )
        .unwrap();
        assert!(
            m.nonfinite_calls > 0,
            "must exercise rejected domain trials"
        );
        assert!(out.warnings.is_empty());
        println!(
            "sqrt guess=1e-4: nodes={} residual={:.17e} calls={}/{} rejected_nonfinite={}",
            out.x.len(),
            out.maxres,
            out.node_evals,
            out.bc_evals,
            m.nonfinite_calls
        );
        for i in 0..21 {
            let x = i as f64 / 20.;
            let (v, _) = evaluate(&out, Method::Lobatto4, x, &mut m);
            assert!((v[0] - (1. + x / 2.).powi(2)).abs() < 2e-5, "{x}: {v:?}");
        }
    }
    fn exponential_probe(method: Method, guess: f64) {
        let mut m = Model::new(7, false);
        let out = solve(
            &mut m,
            method,
            &[0., 0.25, 0.5, 0.75, 1.],
            &vec![vec![guess, 0.]; 5],
            &[],
            &options(1e-3),
        )
        .unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(out.maxres <= 1e-3);
        println!(
            "exp method={} guess={guess}: nodes={} residual={:.17e} calls={}/{} rejected_nonfinite={}",
            if method == Method::Simpson { 4 } else { 5 },
            out.x.len(),
            out.maxres,
            out.node_evals,
            out.bc_evals,
            m.nonfinite_calls
        );
        // y = -2 log(cos(a*(x-1/2))/cos(a/2)), 2*a^2 = cos(a/2)^2.
        let a = 0.668_027_847_453_054_1;
        for i in 0..21 {
            let x = i as f64 / 20.;
            let want = -2. * libm::log(libm::cos(a * (x - 0.5)) / libm::cos(a / 2.));
            let (v, _) = evaluate(&out, method, x, &mut m);
            assert!((v[0] - want).abs() < 2e-5, "{x}: {v:?} vs {want}");
        }
    }
    #[test]
    fn newton_backtracks_exponential_overflow() {
        exponential_probe(Method::Lobatto4, 100.);
    }
    #[test]
    fn newton_freezes_weights_for_shrinking_solution() {
        for method in [Method::Simpson, Method::Lobatto4] {
            exponential_probe(method, 5.);
        }
    }
    #[test]
    fn newton_terminates_at_mesh_rounding_floor() {
        for count in [5, 101, 401, 1001] {
            let mut m = Model::new(0, false);
            let x: Vec<_> = (0..count)
                .map(|i| std::f64::consts::FRAC_PI_2 * i as f64 / (count - 1) as f64)
                .collect();
            let y: Vec<_> = x
                .iter()
                .map(|x| {
                    if count == 5 {
                        vec![0.; 2]
                    } else {
                        vec![libm::sin(*x), libm::cos(*x)]
                    }
                })
                .collect();
            let out = solve(
                &mut m,
                Method::Simpson,
                &x,
                &y,
                &[],
                &BvpOpts {
                    rtol: 1e-10,
                    atol: vec![1e-12],
                    nmax: 1500,
                },
            )
            .unwrap_or_else(|error| panic!("{count} nodes: {error}"));
            assert!(out.warnings.is_empty(), "{count}: {:?}", out.warnings);
            assert!(out.maxres <= 1e-10);
            println!(
                "oscillator initial={count}: nodes={} residual={:.17e} calls={}/{}",
                out.x.len(),
                out.maxres,
                out.node_evals,
                out.bc_evals
            );
            for i in 0..101 {
                let x = std::f64::consts::FRAC_PI_2 * i as f64 / 100.;
                let (v, _) = evaluate(&out, Method::Simpson, x, &mut m);
                assert!((v[0] - libm::sin(x)).abs() < 2e-10, "{count}: {x} {v:?}");
                assert!((v[1] - libm::cos(x)).abs() < 2e-10);
            }
        }
    }
    #[test]
    fn simpson_residual_is_subinterval_l2_norm() {
        // y'=x^3 gives a cubic collocation defect -x(x-h/2)(x-h).
        // Its squared integral is h^7/840. Here atol/rtol=1 so all
        // component weights are exactly one; changing h checks that the
        // residual is an L2 integral, not an RMS or a sampled maximum.
        for h in [1f64, 0.25] {
            for x in [[0., h], [h, 0.]] {
                let mut m = Model::new(8, false);
                let out = solve(
                    &mut m,
                    Method::Simpson,
                    &x,
                    &[vec![0.], vec![0.]],
                    &[],
                    &BvpOpts {
                        rtol: 1e-10,
                        atol: vec![1e-10],
                        nmax: 2,
                    },
                )
                .unwrap();
                let expected = (h.powi(7) / 840.).sqrt();
                assert!(
                    (out.maxres - expected).abs() <= expected * 2e-12,
                    "h={h}: {} vs {expected}",
                    out.maxres
                );
            }
        }
    }
    #[test]
    fn default_oscillator_residual_and_values() {
        let x: Vec<_> = (0..5)
            .map(|i| std::f64::consts::FRAC_PI_2 * i as f64 / 4.)
            .collect();
        for method in [Method::Simpson, Method::Lobatto4] {
            let mut m = Model::new(0, false);
            let out = solve(
                &mut m,
                method,
                &x,
                &vec![vec![0.; 2]; 5],
                &[],
                &options(1e-3),
            )
            .unwrap();
            if method == Method::Simpson {
                assert_eq!(out.x.len(), 7);
                // Independently derived from Simpson's linear update and
                // five-point Lobatto quadrature; see residual_math.py.
                assert!((out.maxres - 0.000_408_478_704_357_033_9).abs() < 2e-14);
            }
            println!(
                "default method={}: nodes={} residual={:.17e} calls={}/{} values={:?} {:?}",
                if method == Method::Simpson { 4 } else { 5 },
                out.x.len(),
                out.maxres,
                out.node_evals,
                out.bc_evals,
                evaluate(&out, method, 0.5, &mut m).0,
                evaluate(&out, method, 1., &mut m).0
            );
        }
    }
    #[test]
    fn trial_errors_preserve_callback_shape_and_cancellation_failures() {
        struct Fault {
            kind: usize,
            trials: usize,
        }
        impl BvpModel for Fault {
            fn f(&mut self, _: f64, y: &[f64], _: &[f64]) -> R<Vec<f64>> {
                if y[0] > 0.5 {
                    self.trials += 1;
                    match self.kind {
                        // Deliberately identical to an internal numerical
                        // message: callback errors must not be swallowed.
                        0 => return Err("BVP collocation equations overflowed.".into()),
                        1 => return Ok(vec![]),
                        _ => return Ok(vec![f64::NAN]),
                    }
                }
                Ok(vec![0.])
            }
            fn bc(&mut self, a: &[f64], _: &[f64], _: &[f64]) -> R<Vec<f64>> {
                Ok(vec![a[0] - 1.])
            }
            fn fjac(&mut self, _: f64, _: &[f64], _: &[f64]) -> R<Option<Vec<f64>>> {
                Ok(Some(vec![0.]))
            }
            fn check_cancel(&self) -> R<()> {
                if self.kind == 2 && self.trials > 0 {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            }
        }
        for (kind, expected) in [
            "BVP collocation equations overflowed.",
            "BVP derivative must be a finite vector of the expected length.",
            "cancelled",
        ]
        .iter()
        .enumerate()
        {
            let mut m = Fault { kind, trials: 0 };
            let error = solve(
                &mut m,
                Method::Lobatto4,
                &[0., 1.],
                &[vec![0.], vec![0.]],
                &[],
                &options(1e-3),
            )
            .err()
            .unwrap();
            assert_eq!(error, *expected);
            assert_eq!(m.trials, 1);
        }
    }
    #[test]
    fn refinement_cap_warning_does_not_claim_a_mesh_limit() {
        let warning = refinement_warning(None, 80, 2e-3, 1e-3);
        assert!(
            warning.starts_with("Warning: Unable to meet the tolerance within 32 mesh solves.")
        );
        assert!(!warning.contains("more than"));
        assert!(warning.contains("last mesh has 80 points"));
        assert!(warning.contains("maximum residual is 2.000e-3, requested tolerance is 1.000e-3"));
    }
    #[test]
    fn bounds_errors_cancellation_and_mesh_limit() {
        for method in [Method::Simpson, Method::Lobatto4] {
            let mut m = Model::new(0, false);
            let mut opts = options(1e-8);
            opts.nmax = 5;
            let x = vec![0., 0.25, 0.5, 0.75, 1.];
            let y = vec![vec![0.; 2]; 5];
            let out = solve(&mut m, method, &x, &y, &[], &opts).unwrap();
            assert_eq!(out.x, x);
            assert_eq!(out.warnings.len(), 1);
            assert!(out.warnings[0].starts_with("Warning: Unable to meet the tolerance without using more than 5 mesh points. The last mesh and solution are returned."));
            assert!(out.warnings[0].contains("The last mesh has 5 points; maximum residual is"));
            assert!(out.maxres > opts.rtol);
            assert!(solve(&mut m, method, &[0., 0.], &y[..2], &[], &opts).is_err());
            assert!(solve(&mut m, method, &[0., f64::INFINITY], &y[..2], &[], &opts).is_err());
            let mut bad = options(1e-3);
            bad.atol = vec![0.];
            assert!(solve(&mut m, method, &x, &y, &[], &bad).is_err());
            let huge = vec![0.; MAX_UNKNOWNS];
            assert!(solve(&mut m, method, &[0., 1.], &[huge.clone(), huge], &[], &opts).is_err());
            m.cancelled.set(true);
            assert_eq!(
                solve(&mut m, method, &x, &y, &[], &opts).err().unwrap(),
                "cancelled"
            );
            let mut during = Model::new(0, false);
            during.cancel_after = Some(5);
            assert_eq!(
                solve(&mut during, method, &x, &y, &[], &opts)
                    .err()
                    .unwrap(),
                "cancelled"
            );
            assert_eq!(during.calls, 5);
            let mut invalid = Model::new(4, false);
            assert!(
                solve(
                    &mut invalid,
                    method,
                    &[0., 1.],
                    &[vec![0.], vec![0.]],
                    &[],
                    &opts
                )
                .is_err()
            );
            let mut singular = Model::new(5, false);
            assert!(
                solve(
                    &mut singular,
                    method,
                    &[0., 1.],
                    &[vec![0.], vec![0.]],
                    &[],
                    &opts
                )
                .err()
                .unwrap()
                .contains("singular Jacobian")
            );
        }
    }
    #[test]
    fn reversed_mesh_and_exact_quadratic() {
        for method in [Method::Simpson, Method::Lobatto4] {
            let mut m = Model::new(3, false);
            let x = vec![1., 0.5, 0.];
            // BC is at the first mesh endpoint: y = x*x - 1.
            let out = solve(
                &mut m,
                method,
                &x,
                &[vec![0.], vec![0.], vec![0.]],
                &[],
                &options(1e-6),
            )
            .unwrap();
            for (x, y) in out.x.iter().zip(&out.y) {
                assert!((y[0] - (x * x - 1.)).abs() < 1e-12);
            }
        }
    }
}
