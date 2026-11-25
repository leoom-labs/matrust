//! One-space-variable method of lines. Spatial formulas and fitted interpolation:
//! Skeel & Berzins, SIAM J. Sci. Stat. Comput. 11 (1990), pp. 3-7,
//! equations (13)-(16), https://doi.org/10.1137/0911001.
//! The singular (origin-including) problem uses quadratic trial functions;
//! regular polar problems use the integral of x^-m. Time integration reuses NDF.
use super::{Engine, Execution, Value, ode_more::ivp};
use crate::{Array, FunctionHandle, Matrix, Result};
use ivp::{JacKind, MassKind, Model, Opts, OutAt, Setup};

const MAX_MESH: usize = 512;
const MAX_STATES: usize = 512;
const MAX_EQUATIONS: usize = 16;
const MAX_TIMES: usize = 10_000;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_RESIDUALS: usize = 50_000;
const MAX_CALLBACKS: usize = 2_000_000;

pub(super) fn handles(name: &str) -> bool {
    matches!(name, "pdepe" | "pdeval")
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > if name == "pdeval" { 2 } else { 1 } {
        return Err(format!("{name}: too many output arguments"));
    }
    match name {
        "pdepe" => solve(e, args, out).map(|v| vec![v]),
        "pdeval" => evaluate(e, args),
        _ => Err(format!("Unknown PDE function {name}")),
    }
}

fn numeric(e: &Engine, v: &Value, label: &str) -> Result<Matrix> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => return Err(format!("{label} must contain real numeric values")),
    };
    e.check_cancel()?;
    if m.is_nd() || m.data().iter().any(|v| !v.is_finite()) {
        return Err(format!("{label} must contain finite real 2-D values"));
    }
    Ok(m)
}

fn vector(e: &Engine, v: &Value, label: &str) -> Result<Matrix> {
    let m = numeric(e, v, label)?;
    if !m.is_empty() && m.rows() != 1 && m.cols() != 1 {
        return Err(format!("{label} must be a vector"));
    }
    Ok(m)
}

fn symmetry(e: &Engine, v: &Value) -> Result<usize> {
    let m = numeric(e, v, "m")?;
    if m.len() != 1 || ![0., 1., 2.].contains(&m.data()[0]) {
        return Err("m must be the scalar 0, 1, or 2".into());
    }
    Ok(m.data()[0] as usize)
}

fn mesh(e: &Engine, v: &Value, m: usize) -> Result<Vec<f64>> {
    let x = vector(e, v, "xmesh")?;
    if x.len() < 3 || x.len() > MAX_MESH {
        return Err(format!("xmesh must have 3 to {MAX_MESH} points"));
    }
    if x.data().windows(2).any(|w| w[0] >= w[1]) {
        return Err("xmesh must be strictly increasing".into());
    }
    if m > 0 && x.data()[0] < 0. {
        return Err("xmesh must be nonnegative when m is 1 or 2".into());
    }
    Ok(x.data().to_vec())
}

fn handle(v: &Value, label: &str) -> Result<FunctionHandle> {
    match v {
        Value::Container(Array::Handle(h)) => Ok(h.clone()),
        _ => Err(format!("{label} must be a function handle")),
    }
}

fn scalar(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}

fn column(x: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(x.len(), 1, x)?))
}

/// Transient callback values release statement accounting, as in the ODE adapters.
fn invoke(
    e: &mut Engine,
    out: &mut Execution,
    h: &FunctionHandle,
    args: Vec<Value>,
    count: usize,
    calls: &mut usize,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    *calls += 1;
    if *calls > MAX_CALLBACKS {
        return Err(format!("pdepe exceeds the {MAX_CALLBACKS} callback limit"));
    }
    let evaluated = e.evaluated_elements;
    let result = e.call_handle(h, args, count, None, out);
    e.evaluated_elements = evaluated;
    let result = result?;
    if result.len() != count {
        return Err(format!("pdepe callback must return {count} outputs"));
    }
    Ok(result)
}

fn callback_vector(e: &Engine, v: &Value, n: Option<usize>, label: &str) -> Result<Vec<f64>> {
    let a = numeric(e, v, label)?;
    if a.cols() != 1 || a.rows() == 0 || n.is_some_and(|n| a.rows() != n) {
        return Err(format!(
            "{label} must be a column vector{}",
            n.map_or_else(String::new, |n| format!(" of length {n}"))
        ));
    }
    Ok(a.data().to_vec())
}

/// Interpolation weights and derivative of the right-node basis function.
/// `singular` describes the whole domain, not just its first element (paper 2.2).
fn basis(m: usize, singular: bool, a: f64, b: f64, q: f64) -> (f64, f64) {
    let h = b - a;
    if singular {
        let denom = h * (a + b);
        ((q - a) * (q + a) / denom, 2. * q / denom)
    } else {
        match m {
            0 => ((q - a) / h, 1. / h),
            1 => {
                let log = (h / a).ln_1p();
                (((q - a) / a).ln_1p() / log, 1. / (q * log))
            }
            _ => (b / q * ((q - a) / h), (a / q) * (b / q) / h),
        }
    }
}

struct Element {
    x: f64,
    right: f64,
    derivative: f64,
    flux: f64,
    left_mass: f64,
    right_mass: f64,
}

fn elements(m: usize, x: &[f64]) -> Result<Vec<Element>> {
    let singular = m > 0 && x[0] == 0.;
    x.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let h = b - a;
            let mid = a + h / 2.;
            // z = zeta^(m+1), paper (13). Use a series for the cancellation in
            // the logarithmic mean when an annular element is very narrow.
            let (z, left_mass, right_mass) = match m {
                0 => (mid, h / 2., h / 2.),
                1 if a == 0. => (0., 0., b * b / 2.),
                1 => {
                    let z = mid * h / (h / a).ln_1p();
                    let left = if h / a < 1e-5 {
                        a * h / 2. + h * h / 12.
                    } else {
                        (z - a * a) / 2.
                    };
                    (z, left, h * mid - left)
                }
                _ => {
                    let left = a * a * h / 2. + a * h * h / 6.;
                    let right = a * a * h / 2. + 5. * a * h * h / 6. + h * h * h / 3.;
                    (a * b * mid, left, right)
                }
            };
            // One-point quadrature: gamma_1 in a singular domain, gamma_-m otherwise.
            let q = if singular {
                mid + h * (h / mid) / 12.
            } else {
                match m {
                    0 => mid,
                    1 => h / (h / a).ln_1p(),
                    _ => a * (b / h) * (h / a).ln_1p(),
                }
            };
            let flux = if singular { z / q } else { q.powi(m as i32) };
            let (right, derivative) = basis(m, singular, a, b, q);
            if ![q, right, derivative, flux, left_mass, right_mass]
                .iter()
                .all(|v| v.is_finite())
                || q <= a
                || q >= b
                || derivative <= 0.
                || left_mass < 0.
                || right_mass <= 0.
                || (a != 0. && left_mass == 0.)
            {
                return Err(
                    "pdepe mesh spacing is too small or too large for stable discretization".into(),
                );
            }
            Ok(Element {
                x: q,
                right,
                derivative,
                flux,
                left_mass,
                right_mass,
            })
        })
        .collect()
}

struct Problem<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    pde: FunctionHandle,
    bc: FunctionHandle,
    m: usize,
    x: Vec<f64>,
    elements: Vec<Element>,
    npde: usize,
    calls: usize,
    residuals: usize,
    /// Normalizing positive c leaves a constant diagonal 0/1 mass matrix.
    differential: Option<Vec<bool>>,
    dirichlet: Option<Vec<bool>>,
}

impl Problem<'_> {
    fn residual(&mut self, t: f64, y: &[f64]) -> Result<Vec<f64>> {
        self.e.check_cancel()?;
        self.residuals += 1;
        if self.residuals > MAX_RESIDUALS {
            return Err(format!(
                "pdepe exceeds the {MAX_RESIDUALS} residual/step limit"
            ));
        }
        if y.iter().any(|v| !v.is_finite()) {
            return Err("pdepe encountered a nonfinite solution".into());
        }
        let n = self.npde;
        let nx = self.x.len();
        let mut mass = vec![0.; y.len()];
        let mut rhs = vec![0.; y.len()];
        let mut volume = vec![0.; nx];
        for j in 0..nx - 1 {
            let el = &self.elements[j];
            let mut u = vec![0.; n];
            let mut du = vec![0.; n];
            for k in 0..n {
                let delta = y[(j + 1) * n + k] - y[j * n + k];
                u[k] = y[j * n + k] + el.right * delta;
                du[k] = el.derivative * delta;
            }
            let values = invoke(
                self.e,
                self.out,
                &self.pde,
                vec![scalar(el.x), scalar(t), column(&u)?, column(&du)?],
                3,
                &mut self.calls,
            )?;
            let c = callback_vector(self.e, &values[0], Some(n), "pdefun c")?;
            let f = callback_vector(self.e, &values[1], Some(n), "pdefun f")?;
            let s = callback_vector(self.e, &values[2], Some(n), "pdefun s")?;
            if c.iter().any(|v| *v < 0.) {
                return Err("pdefun c must be nonnegative (zero for elliptic components)".into());
            }
            for k in 0..n {
                let l = j * n + k;
                let r = (j + 1) * n + k;
                mass[l] += el.left_mass * c[k];
                mass[r] += el.right_mass * c[k];
                rhs[l] += el.flux * f[k] + el.left_mass * s[k];
                rhs[r] += -el.flux * f[k] + el.right_mass * s[k];
                if j == 0 && self.m > 0 && self.x[0] == 0. {
                    // Limit of (15)/zeta^(m+1) at the origin, paper p. 7.
                    mass[l] = c[k];
                    rhs[l] = (self.m + 1) as f64 * f[k] / el.x + s[k];
                }
            }
            volume[j] += el.left_mass;
            volume[j + 1] += el.right_mass;
        }
        let values = invoke(
            self.e,
            self.out,
            &self.bc,
            vec![
                scalar(self.x[0]),
                column(&y[..n])?,
                scalar(self.x[nx - 1]),
                column(&y[(nx - 1) * n..])?,
                scalar(t),
            ],
            4,
            &mut self.calls,
        )?;
        let origin = self.m > 0 && self.x[0] == 0.;
        let (pl, ql) = if origin {
            (vec![0.; n], vec![0.; n])
        } else {
            (
                callback_vector(self.e, &values[0], Some(n), "bcfun pl")?,
                callback_vector(self.e, &values[1], Some(n), "bcfun ql")?,
            )
        };
        let pr = callback_vector(self.e, &values[2], Some(n), "bcfun pr")?;
        let qr = callback_vector(self.e, &values[3], Some(n), "bcfun qr")?;
        let dirichlet: Vec<_> = ql
            .iter()
            .map(|q| !(self.m > 0 && self.x[0] == 0.) && *q == 0.)
            .chain(qr.iter().map(|q| *q == 0.))
            .collect();
        if self
            .dirichlet
            .as_ref()
            .is_some_and(|previous| *previous != dirichlet)
        {
            return Err("pdepe requires a fixed zero/nonzero boundary q pattern".into());
        }
        self.dirichlet = Some(dirichlet);
        for (j, p, q, sign) in [(0, &pl, &ql, 1.), (nx - 1, &pr, &qr, -1.)] {
            if j == 0 && self.m > 0 && self.x[0] == 0. {
                volume[0] = 1.; // Left BC is ignored at the coordinate singularity.
                continue;
            }
            for k in 0..n {
                let i = j * n + k;
                if q[k] == 0. {
                    mass[i] = 0.;
                    rhs[i] = -p[k];
                } else {
                    rhs[i] += sign * self.x[j].powi(self.m as i32) * p[k] / q[k];
                }
            }
        }
        let kind: Vec<_> = mass.iter().map(|v| *v > 0.).collect();
        if let Some(previous) = &self.differential {
            if *previous != kind {
                return Err(
                    "pdepe requires a fixed parabolic/elliptic and zero/nonzero boundary q pattern"
                        .into(),
                );
            }
        } else {
            if !kind.iter().any(|v| *v) {
                return Err("pdepe requires at least one parabolic equation".into());
            }
            self.differential = Some(kind);
        }
        for i in 0..rhs.len() {
            // Differential rows are divided by their positive c-weighted volume.
            // Algebraic rows keep a zero mass; scale interior constraints by volume.
            let j = i / n;
            if mass[i] > 0. {
                rhs[i] /= mass[i];
            } else if j != 0 && j != nx - 1 {
                rhs[i] /= volume[j];
            }
        }
        if mass.iter().chain(&rhs).any(|v| !v.is_finite()) {
            return Err("pdepe discretization produced nonfinite coefficients".into());
        }
        Ok(rhs)
    }
}

impl Model for Problem<'_> {
    fn f(&mut self, t: f64, y: &[f64]) -> Result<Vec<f64>> {
        self.residual(t, y)
    }
    fn jac(&mut self, _: f64, _: &[f64]) -> Result<Vec<f64>> {
        Err("pdepe uses a numerical Jacobian".into())
    }
    fn mass(&mut self, _: f64, _: &[f64]) -> Result<Vec<f64>> {
        Err("pdepe uses a constant normalized mass matrix".into())
    }
    fn events(&mut self, _: f64, _: &[f64]) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        Err("pdepe Events is not supported yet".into())
    }
    fn check_cancel(&self) -> Result<()> {
        self.e.check_cancel()
    }
}

fn tolerances(e: &Engine, value: Option<&Value>, n: usize) -> Result<(f64, Vec<f64>)> {
    let mut rtol = 1e-3;
    let mut atol = vec![1e-6; n];
    let Some(value) = value else {
        return Ok((rtol, atol));
    };
    if matches!(value, Value::Matrix(a) if a.is_empty()) {
        return Ok((rtol, atol));
    }
    let Value::Container(Array::Struct(s)) = value else {
        return Err("pdepe options must be an odeset scalar structure or []".into());
    };
    if s.len() != 1 {
        return Err("pdepe options must be a scalar structure".into());
    }
    for (i, name) in s.field_names().iter().enumerate() {
        let a = s.get(i, 0)?;
        if a.is_empty() {
            continue;
        }
        match name.as_str() {
            "RelTol" => {
                let v = numeric(e, &Value::Container((**a).clone()), "RelTol")?;
                if v.len() != 1 || v.data()[0] <= 0. {
                    return Err("pdepe RelTol must be a positive finite scalar".into());
                }
                rtol = v.data()[0].max(100. * f64::EPSILON);
            }
            "AbsTol" => {
                let v = vector(e, &Value::Container((**a).clone()), "AbsTol")?;
                if (v.len() != 1 && v.len() != n) || v.data().iter().any(|v| *v <= 0.) {
                    return Err(format!(
                        "pdepe AbsTol must be a positive scalar or vector of length {n}"
                    ));
                }
                for (k, a) in atol.iter_mut().enumerate() {
                    *a = v.data()[k % v.len()];
                }
            }
            _ => {
                return Err(format!(
                    "pdepe option {name} is not supported yet; use RelTol and AbsTol"
                ));
            }
        }
    }
    Ok((rtol, atol))
}

fn solve(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if !(6..=7).contains(&args.len()) {
        return Err(
            "pdepe expects m, pdefun, icfun, bcfun, xmesh, tspan, and optional options".into(),
        );
    }
    let m = symmetry(e, &args[0])?;
    let pde = handle(&args[1], "pdefun")?;
    let ic = handle(&args[2], "icfun")?;
    let bc = handle(&args[3], "bcfun")?;
    let x = mesh(e, &args[4], m)?;
    let elements = elements(m, &x)?;
    let times = vector(e, &args[5], "tspan")?;
    // Two endpoints are accepted as well as the documented >=3 point form.
    if times.len() < 2 || times.len() > MAX_TIMES || times.data().windows(2).any(|w| w[0] >= w[1]) {
        return Err(format!(
            "tspan must contain 2 to {MAX_TIMES} strictly increasing times"
        ));
    }
    let t = times.data();
    let duration = t[t.len() - 1] - t[0];
    if !duration.is_finite() || duration <= 0. {
        return Err("pdepe time interval is too large".into());
    }
    let mut calls = 0;
    let initial = invoke(e, out, &ic, vec![scalar(x[0])], 1, &mut calls)?;
    let first = callback_vector(e, &initial[0], None, "icfun output")?;
    let n = first.len();
    if n > MAX_EQUATIONS || x.len() * n > MAX_STATES {
        return Err(format!(
            "pdepe exceeds {MAX_EQUATIONS} equations or {MAX_STATES} mesh states"
        ));
    }
    let neq = x.len() * n;
    if neq * t.len() > MAX_OUTPUT {
        return Err(format!(
            "pdepe output exceeds the {MAX_OUTPUT}-element limit"
        ));
    }
    let (rtol, abs) = tolerances(e, args.get(6), n)?;
    let atol: Vec<_> = (0..x.len()).flat_map(|_| abs.iter().copied()).collect();
    let mut y0 = Vec::with_capacity(neq);
    y0.extend(first);
    for &point in x.iter().skip(1) {
        let initial = invoke(e, out, &ic, vec![scalar(point)], 1, &mut calls)?;
        y0.extend(callback_vector(e, &initial[0], Some(n), "icfun output")?);
    }
    let mut problem = Problem {
        e,
        out,
        pde,
        bc,
        m,
        x,
        elements,
        npde: n,
        calls,
        residuals: 0,
        differential: None,
        dirichlet: None,
    };
    let f0 = problem.residual(t[0], &y0)?;
    let mask = problem.differential.as_ref().unwrap();
    let mut mass = vec![0.; neq * neq];
    for i in 0..neq {
        mass[i * neq + i] = f64::from(mask[i]);
    }
    let options = Opts {
        rtol,
        threshold: atol.iter().map(|a| a / rtol).collect(),
        atol,
        normcontrol: false,
        hmax: duration / 10.,
        htry: None,
        htspan: t[1] - t[0],
        maxorder: 5,
        bdf: false,
        jac: JacKind::Numeric,
        jconstant: false,
        mass: MassKind::Matrix(mass),
        mass_singular: if mask.iter().all(|v| *v) { "no" } else { "yes" }.into(),
        initial_slope: None,
        events: false,
        non_negative: Vec::new(),
    };
    let result = ivp::ode15s(
        &mut problem,
        Setup {
            tspan: t,
            y0: &y0,
            f0,
            valt: None,
            m0: None,
            at: OutAt::Requested,
        },
        &options,
    )?;
    if result.failure.is_some() || result.t.len() != t.len() {
        return Err(format!(
            "pdepe time integration failed before the final requested time: {}",
            result.warnings.join(" ")
        ));
    }
    problem.e.check_cancel()?;
    let nt = t.len();
    let nx = problem.x.len();
    let mut data = vec![0.; nt * nx * n];
    for k in 0..n {
        for j in 0..nx {
            problem.e.check_cancel()?;
            for i in 0..nt {
                let value = result.y[i][j * n + k];
                if !value.is_finite() {
                    return Err("pdepe time interpolation produced a nonfinite solution".into());
                }
                data[i + nt * (j + nx * k)] = value;
            }
        }
    }
    Ok(Value::Matrix(
        Matrix::from_column_vec(nt, nx * n, data)?.with_dims(&[nt, nx, n])?,
    ))
}

fn evaluate(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() != 4 {
        return Err("pdeval expects m, xmesh, ui, and xout".into());
    }
    let m = symmetry(e, &args[0])?;
    let x = mesh(e, &args[1], m)?;
    elements(m, &x)?; // Accept the same stable mesh geometry as pdepe.
    let ui = vector(e, &args[2], "ui")?;
    if ui.len() != x.len() {
        return Err("pdeval ui must have one value per xmesh point".into());
    }
    let q = vector(e, &args[3], "xout")?;
    if q.len() > MAX_OUTPUT || q.data().iter().any(|v| *v < x[0] || *v > x[x.len() - 1]) {
        return Err("pdeval xout must lie within xmesh and satisfy the output limit".into());
    }
    let singular = m > 0 && x[0] == 0.;
    let mut u = Vec::with_capacity(q.len());
    let mut du = Vec::with_capacity(q.len());
    for (i, &point) in q.data().iter().enumerate() {
        if i % 1024 == 0 {
            e.check_cancel()?;
        }
        let j = x
            .partition_point(|&v| v <= point)
            .saturating_sub(1)
            .min(x.len() - 2);
        let (r, d) = basis(m, singular, x[j], x[j + 1], point);
        let delta = ui.data()[j + 1] - ui.data()[j];
        // Preserve mesh values exactly, including a tiny endpoint next to a
        // much larger value. Evaluate from the nearer endpoint in between.
        u.push(if point == x[j] {
            ui.data()[j]
        } else if point == x[j + 1] {
            ui.data()[j + 1]
        } else if r <= 0.5 {
            ui.data()[j] + r * delta
        } else {
            ui.data()[j + 1] - (1. - r) * delta
        });
        du.push(d * delta);
    }
    if u.iter().chain(&du).any(|v| !v.is_finite()) {
        return Err(
            "pdeval mesh spacing is too small or too large for stable interpolation".into(),
        );
    }
    Ok(vec![
        Value::Matrix(Matrix::from_column_vec(q.rows(), q.cols(), u)?),
        Value::Matrix(Matrix::from_column_vec(q.rows(), q.cols(), du)?),
    ])
}
