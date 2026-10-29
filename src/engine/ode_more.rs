//! ode113, ode15s, ode23s, ode23t, ode23tb, ode15i, decic, deval, odextend,
//! bvp4c, bvp5c, bvpinit, bvpget, bvpset, bvpxtend, dde23, ddeget, ddeset and
//! ddesd: argument handling, option sets, solution structures and output in
//! the reference's shapes. The numerics live in the engine-independent
//! submodules; problem functions are called back through the engine with the
//! statement budget released after every call.
mod bvp;
mod dde;
mod implicit;
mod ivp;
mod num;

use super::{Engine, Execution, Value};
use crate::{Array, CellArray, CharacterRow, FunctionHandle, Matrix, Result, StructArray};
use std::sync::Arc;

use ivp::{JacKind, MassKind, Model, OutAt, Setup, StepData};
use num::fmt_g;

const SOLVERS: [&str; 6] = ["ode113", "ode15s", "ode23s", "ode23t", "ode23tb", "ode15i"];

/// Names handled here.
pub(super) fn handles(name: &str) -> bool {
    SOLVERS.contains(&name)
        || matches!(
            name,
            "decic"
                | "deval"
                | "odextend"
                | "bvp4c"
                | "bvp5c"
                | "bvpinit"
                | "bvpget"
                | "bvpset"
                | "bvpxtend"
                | "dde23"
                | "ddeget"
                | "ddeset"
                | "ddesd"
        )
}

/// Functions with more than one output.
pub(super) fn has_outputs(name: &str) -> bool {
    SOLVERS.contains(&name) || matches!(name, "decic" | "deval")
}

const BVP_NAMES: [&str; 8] = [
    "AbsTol",
    "RelTol",
    "SingularTerm",
    "FJacobian",
    "BCJacobian",
    "Stats",
    "Nmax",
    "Vectorized",
];

const DDE_NAMES: [&str; 12] = [
    "AbsTol",
    "Events",
    "InitialStep",
    "InitialY",
    "Jumps",
    "MaxStep",
    "NormControl",
    "OutputFcn",
    "OutputSel",
    "Refine",
    "RelTol",
    "Stats",
];

/// A problem function: a handle or a function name.
#[derive(Clone)]
enum Callable {
    Handle(FunctionHandle),
    Name(String),
}

impl Callable {
    /// Function text for diagnostics: the uppercased handle text or the name.
    fn diagnostic(&self) -> String {
        match self {
            Self::Handle(h) => h.func2str().to_uppercase(),
            Self::Name(n) => n.clone(),
        }
    }
    fn value(&self) -> Array {
        match self {
            Self::Handle(h) => Array::Handle(h.clone()),
            Self::Name(n) => Array::Handle(FunctionHandle::named(n)),
        }
    }
}

fn empty() -> Array {
    Array::Numeric(Matrix::filled(0, 0, 0.).unwrap_or_else(|_| Matrix::scalar(0.)))
}

fn num(v: f64) -> Arc<Array> {
    Arc::new(Array::Numeric(Matrix::scalar(v)))
}

fn row(values: Vec<f64>) -> Result<Array> {
    let n = values.len();
    Ok(Array::Numeric(Matrix::from_column_vec(1, n, values)?))
}

fn column(values: Vec<f64>) -> Result<Array> {
    let n = values.len();
    Ok(Array::Numeric(Matrix::from_column_vec(n, 1, values)?))
}

/// neq-by-n matrix from column vectors.
fn columns(cols: &[Vec<f64>], neq: usize) -> Result<Array> {
    let mut data = Vec::with_capacity(Matrix::checked_len(neq, cols.len())?);
    for c in cols {
        data.extend_from_slice(c);
    }
    Ok(Array::Numeric(Matrix::from_column_vec(
        neq,
        cols.len(),
        data,
    )?))
}

/// n-by-neq matrix with one row per point (the [t, y] layout).
fn rows_of(points: &[Vec<f64>], neq: usize) -> Result<Array> {
    let n = points.len();
    let mut data = Vec::with_capacity(Matrix::checked_len(n, neq)?);
    for j in 0..neq {
        data.extend(points.iter().map(|p| p[j]));
    }
    Ok(Array::Numeric(Matrix::from_column_vec(n, neq, data)?))
}

fn text(s: &str) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(
        s,
        &std::sync::atomic::AtomicBool::new(false),
    )?))
}

fn upper(name: &str) -> String {
    name.to_uppercase()
}

/// Canonical option name for a case-insensitive prefix.
fn property(names: &[&'static str], name: &str) -> Result<&'static str> {
    let lower = name.to_lowercase();
    let matches: Vec<&'static str> = names
        .iter()
        .copied()
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .collect();
    match matches[..] {
        [] => Err(format!("Unrecognized property name '{name}'.")),
        [only] => Ok(only),
        _ => match matches.iter().find(|n| n.to_lowercase() == lower) {
            Some(exact) => Ok(exact),
            None => Err(format!(
                "Ambiguous property name '{name}' ({}).",
                matches.join(", ")
            )),
        },
    }
}

/// The interpolants deval uses, with their derivatives.
mod interp {
    use super::implicit::lagrange;
    use super::ivp::{ntrp15s, ntrp23s, ntrp23t, ntrp23tb, ntrp113};

    pub fn slope(f: &dyn Fn(f64) -> Vec<f64>, x: f64, scale: f64) -> Vec<f64> {
        // Symmetric differences for an arbitrary continuous extension; scale
        // with |h| so reverse-time interpolation has the same accuracy.
        // Differentiate in the extension's local time scale. A floor of one
        // would extrapolate far outside very short steps near the origin.
        let delta = (libm::cbrt(f64::EPSILON) * scale.abs().max(f64::MIN_POSITIVE))
            .max(4. * super::num::eps_of(x));
        let left = x - delta;
        let right = x + delta;
        f(right)
            .iter()
            .zip(f(left))
            .map(|(a, b)| (a - b) / (right - left))
            .collect()
    }

    pub fn rk(
        bi: &[[f64; 4]],
        stages: &[&[f64]],
        t: f64,
        y: &[f64],
        h: f64,
        x: f64,
    ) -> (Vec<f64>, Vec<f64>) {
        // Continuous RK extension p(s)=y+h*sum_i k_i*sum_j b_ij*s^(j+1).
        // Hairer, Norsett & Wanner (1993), section II.6.
        let s = (x - t) / h;
        let mut value = y.to_vec();
        let mut derivative = vec![0.; y.len()];
        for (i, initial) in y.iter().enumerate() {
            let mut v = 0.;
            let mut d = 0.;
            for (coefficients, stage) in bi.iter().zip(stages) {
                let b = coefficients;
                let polynomial = s * (b[0] + s * (b[1] + s * (b[2] + s * b[3])));
                let differentiated = b[0] + s * (2. * b[1] + s * (3. * b[2] + s * 4. * b[3]));
                v += stage[i] * polynomial;
                d += stage[i] * differentiated;
            }
            value[i] = initial + h * v;
            derivative[i] = d;
        }
        (value, derivative)
    }

    pub fn abm(
        x: f64,
        tnew: f64,
        ynew: &[f64],
        k: usize,
        phi: &[Vec<f64>],
        psi: &[f64],
    ) -> (Vec<f64>, Vec<f64>) {
        // Continuous extensions: Hairer, Norsett & Wanner (1993), sections
        // II.6, III.1 and III.5. Retain the documented symmetric difference
        // for the derivative of this extension.
        let extension = |x| ntrp113(x, tnew, ynew, k, phi, psi);
        (extension(x), slope(&extension, x, psi[1]))
    }

    pub fn ndf(
        x: f64,
        tnew: f64,
        ynew: &[f64],
        h: f64,
        dif: &[Vec<f64>],
        k: usize,
    ) -> (Vec<f64>, Vec<f64>) {
        // Continuous extensions: Hairer, Norsett & Wanner (1993), sections
        // II.6, III.1 and III.5. Retain the documented symmetric difference
        // for the derivative of this extension.
        let extension = |x| ntrp15s(x, tnew, ynew, h, dif, k);
        (extension(x), slope(&extension, x, h))
    }

    pub fn ros(x: f64, t: f64, y: &[f64], h: f64, k1: &[f64], k2: &[f64]) -> (Vec<f64>, Vec<f64>) {
        // Modified Rosenbrock extension: Shampine & Reichelt, The MATLAB
        // ODE Suite (1997), section 4. Retain the documented symmetric
        // difference for the derivative of this extension.
        let extension = |x| ntrp23s(x, t, y, h, k1, k2);
        (extension(x), slope(&extension, x, h))
    }

    pub fn tr(
        x: f64,
        t: f64,
        y: &[f64],
        h: f64,
        ynew: &[f64],
        z: &[f64],
        znew: &[f64],
    ) -> (Vec<f64>, Vec<f64>) {
        // Continuous extensions: Hairer, Norsett & Wanner (1993), sections
        // II.6, III.1 and III.5. Retain the documented symmetric difference
        // for the derivative of this extension.
        let extension = |x| ntrp23t(x, t, y, h, ynew, z, znew);
        (extension(x), slope(&extension, x, h))
    }

    pub fn trbdf(
        x: f64,
        t: f64,
        y: &[f64],
        tnew: f64,
        ynew: &[f64],
        t2: f64,
        y2: &[f64],
    ) -> (Vec<f64>, Vec<f64>) {
        // Continuous extensions: Hairer, Norsett & Wanner (1993), sections
        // II.6, III.1 and III.5. Retain the documented symmetric difference
        // for the derivative of this extension.
        let extension = |x| ntrp23tb(x, t, y, tnew, ynew, t2, y2);
        (extension(x), slope(&extension, x, tnew - t))
    }

    pub fn poly(x: f64, ts: &[f64], ys: &[&[f64]]) -> (Vec<f64>, Vec<f64>) {
        // Continuous extensions: Hairer, Norsett & Wanner (1993), sections
        // II.6, III.1 and III.5. Retain the documented symmetric difference
        // for the derivative of this extension.
        let extension = |x| lagrange(ts, ys, x);
        (extension(x), slope(&extension, x, ts[ts.len() - 1] - ts[0]))
    }
}

/// The IVP model the solvers call back.
struct Ivp<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    f: Callable,
    extras: Vec<Value>,
    jac: Option<Callable>,
    mass: Option<(Callable, bool)>,
    events: Option<Callable>,
    neq: usize,
}

impl Ivp<'_> {
    fn args(&self, mut head: Vec<Value>) -> Vec<Value> {
        head.extend(self.extras.iter().cloned());
        head
    }
}

fn tv(t: f64) -> Value {
    Value::Matrix(Matrix::scalar(t))
}

fn cv(y: &[f64]) -> Value {
    Value::Matrix(Matrix::from_column_slice(y.len(), 1, y).unwrap_or_else(|_| Matrix::scalar(0.)))
}

impl Model for Ivp<'_> {
    fn f(&mut self, t: f64, y: &[f64]) -> num::R<Vec<f64>> {
        let args = self.args(vec![tv(t), cv(y)]);
        let f = self.f.clone();
        let v = self.e.od2_invoke(&f, args, 1, self.out)?;
        self.e.od2_derivative(v, &f, self.neq)
    }
    fn jac(&mut self, t: f64, y: &[f64]) -> num::R<Vec<f64>> {
        let args = self.args(vec![tv(t), cv(y)]);
        let Some(j) = self.jac.clone() else {
            return Err("No Jacobian function".into());
        };
        let v = self.e.od2_invoke(&j, args, 1, self.out)?;
        self.e.od2_square(v, self.neq)
    }
    fn mass(&mut self, t: f64, y: &[f64]) -> num::R<Vec<f64>> {
        let Some((mfun, state)) = self.mass.clone() else {
            return Ok(ivp::identity(self.neq));
        };
        let head = if state {
            vec![tv(t), cv(y)]
        } else {
            vec![tv(t)]
        };
        let args = self.args(head);
        let v = self.e.od2_invoke(&mfun, args, 1, self.out)?;
        self.e.od2_square(v, self.neq)
    }
    fn events(&mut self, t: f64, y: &[f64]) -> num::R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        let Some(ev) = self.events.clone() else {
            return Err("No event function".into());
        };
        let args = self.args(vec![tv(t), cv(y)]);
        self.e.od2_event_values(&ev, args, self.out)
    }
    fn check_cancel(&self) -> num::R<()> {
        self.e.check_cancel()
    }
}

struct ImplicitIvp<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    f: Callable,
    extras: Vec<Value>,
    neq: usize,
}

impl implicit::Implicit for ImplicitIvp<'_> {
    fn f(&mut self, t: f64, y: &[f64], yp: &[f64]) -> num::R<Vec<f64>> {
        let mut args = vec![tv(t), cv(y), cv(yp)];
        args.extend(self.extras.iter().cloned());
        let f = self.f.clone();
        let v = self.e.od2_invoke(&f, args, 1, self.out)?;
        self.e.od2_derivative(v, &f, self.neq)
    }
    fn check_cancel(&self) -> num::R<()> {
        self.e.check_cancel()
    }
}

struct BvpCalls<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    f: Callable,
    bc: Callable,
    fjac: Option<Callable>,
    extras: Vec<Value>,
    neq: usize,
    npar: usize,
    solver: String,
}

impl bvp::BvpModel for BvpCalls<'_> {
    fn f(&mut self, x: f64, y: &[f64], p: &[f64]) -> num::R<Vec<f64>> {
        let mut args = vec![tv(x), cv(y)];
        if self.npar > 0 {
            args.push(cv(p));
        }
        args.extend(self.extras.iter().cloned());
        let f = self.f.clone();
        let v = self.e.od2_invoke(&f, args, 1, self.out)?;
        let v = self
            .e
            .od2_vector(v.into_iter().next().unwrap_or(Value::Void))?;
        if v.len() != self.neq {
            return Err(format!(
                "Error in calling {}(ODEFUN,BCFUN,SOLINIT): \n  The derivative function ODEFUN should return a column vector of length {}.",
                upper(&self.solver),
                self.neq
            ));
        }
        Ok(v)
    }
    fn bc(&mut self, ya: &[f64], yb: &[f64], p: &[f64]) -> num::R<Vec<f64>> {
        let mut args = vec![cv(ya), cv(yb)];
        if self.npar > 0 {
            args.push(cv(p));
        }
        args.extend(self.extras.iter().cloned());
        let bc = self.bc.clone();
        let v = self.e.od2_invoke(&bc, args, 1, self.out)?;
        let v = self
            .e
            .od2_vector(v.into_iter().next().unwrap_or(Value::Void))?;
        if v.len() != self.neq + self.npar {
            return Err(format!(
                "Error in calling {}(ODEFUN,BCFUN,SOLINIT): \n  The boundary condition function BCFUN should return a column vector of length {}.",
                upper(&self.solver),
                self.neq + self.npar
            ));
        }
        Ok(v)
    }
    fn fjac(&mut self, x: f64, y: &[f64], p: &[f64]) -> num::R<Option<Vec<f64>>> {
        let Some(j) = self.fjac.clone() else {
            return Ok(None);
        };
        let mut args = vec![tv(x), cv(y)];
        if self.npar > 0 {
            args.push(cv(p));
        }
        args.extend(self.extras.iter().cloned());
        let v = self.e.od2_invoke(&j, args, 1, self.out)?;
        Ok(Some(self.e.od2_square(v, self.neq)?))
    }
    fn check_cancel(&self) -> num::R<()> {
        self.e.check_cancel()
    }
}

struct DdeCalls<'a> {
    e: &'a mut Engine,
    out: &'a mut Execution,
    f: Callable,
    history: Option<Callable>,
    history_solution: Option<StructArray>,
    delays: Option<Callable>,
    events: Option<Callable>,
    extras: Vec<Value>,
    neq: usize,
    ndel: usize,
}

impl dde::DdeModel for DdeCalls<'_> {
    fn f(&mut self, t: f64, y: &[f64], z: &[f64]) -> num::R<Vec<f64>> {
        let zm = Value::Matrix(Matrix::from_column_slice(self.neq, self.ndel, z)?);
        let mut args = vec![tv(t), cv(y), zm];
        args.extend(self.extras.iter().cloned());
        let f = self.f.clone();
        let v = self.e.od2_invoke(&f, args, 1, self.out)?;
        self.e.od2_derivative(v, &f, self.neq)
    }
    fn history(&mut self, t: f64) -> num::R<Vec<f64>> {
        if let Some(sol) = self.history_solution.clone() {
            return self.e.od2_history_value(sol, t, &self.extras, self.out);
        }
        let Some(h) = self.history.clone() else {
            return Err("No history function".into());
        };
        let mut args = vec![tv(t)];
        args.extend(self.extras.iter().cloned());
        let v = self.e.od2_invoke(&h, args, 1, self.out)?;
        self.e
            .od2_vector(v.into_iter().next().unwrap_or(Value::Void))
    }
    fn delays(&mut self, t: f64, y: &[f64]) -> num::R<Vec<f64>> {
        let Some(d) = self.delays.clone() else {
            return Err("No delay function".into());
        };
        let mut args = vec![tv(t), cv(y)];
        args.extend(self.extras.iter().cloned());
        let v = self.e.od2_invoke(&d, args, 1, self.out)?;
        self.e
            .od2_vector(v.into_iter().next().unwrap_or(Value::Void))
    }
    fn initial_events(&mut self, t: f64, y: &[f64], z: &[f64]) -> num::R<()> {
        let ev = self.events.clone().ok_or("No event function")?;
        let zm = Value::Matrix(Matrix::from_column_slice(self.neq, self.ndel, z)?);
        let mut args = vec![tv(t), cv(y), zm];
        args.extend(self.extras.iter().cloned());
        self.e.od2_invoke(&ev, args, 1, self.out)?;
        Ok(())
    }
    fn events(&mut self, t: f64, y: &[f64], z: &[f64]) -> num::R<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        let Some(ev) = self.events.clone() else {
            return Err("No event function".into());
        };
        let zm = Value::Matrix(Matrix::from_column_slice(self.neq, self.ndel, z)?);
        let mut args = vec![tv(t), cv(y), zm];
        args.extend(self.extras.iter().cloned());
        self.e.od2_event_values(&ev, args, self.out)
    }
    fn check_cancel(&self) -> num::R<()> {
        self.e.check_cancel()
    }
}

/// Options read from an odeset/bvpset/ddeset struct: non-empty fields by name.
struct OptionSet {
    set: Option<StructArray>,
}

impl OptionSet {
    fn get(&self, name: &str) -> Option<Arc<Array>> {
        let set = self.set.as_ref()?;
        let field = set.field_index(name)?;
        let value = set.get(field, 0).ok()?;
        (!value.is_empty()).then(|| Arc::clone(value))
    }
}

impl Engine {
    /// Single-output calls (`x = name(...)` and expressions).
    pub(super) fn od2_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if !handles(name) {
            return Ok(None);
        }
        let mut values = self.od2_dispatch(name, args.to_vec(), 1, out)?;
        Ok(Some(if values.is_empty() {
            Value::Void
        } else {
            values.remove(0)
        }))
    }

    /// Multiple outputs from eval_outputs.
    pub(super) fn od2_outputs(
        &mut self,
        name: &str,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let values = self.od2_dispatch(name, args, count, out)?;
        if values.len() < count {
            return Err("Too many output arguments.".into());
        }
        Ok(values)
    }

    fn od2_dispatch(
        &mut self,
        name: &str,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        match name {
            "ode15i" => self.od2_ode15i(args, count, out),
            _ if SOLVERS.contains(&name) => self.od2_ivp(name, args, count, out),
            "decic" => self.od2_decic(args, count, out),
            "deval" => self.od2_deval(args, count),
            "odextend" => Ok(vec![self.od2_odextend(args, out)?]),
            "bvpinit" => Ok(vec![self.od2_bvpinit(args, out)?]),
            "bvpset" => Ok(vec![self.od2_set(&BVP_NAMES, "BVPSET", args)?]),
            "ddeset" => Ok(vec![self.od2_set(&DDE_NAMES, "DDESET", args)?]),
            "bvpget" => Ok(vec![self.od2_get(&BVP_NAMES, "BVPSET", args)?]),
            "ddeget" => Ok(vec![self.od2_get(&DDE_NAMES, "DDESET", args)?]),
            "bvp4c" | "bvp5c" => Ok(vec![self.od2_bvp(name, args, out)?]),
            "bvpxtend" => Ok(vec![self.od2_bvpxtend(args)?]),
            "dde23" | "ddesd" => Ok(vec![self.od2_dde(name, args, out)?]),
            _ => Err(format!("{name} is not supported")),
        }
    }

    /// Calls a problem function and releases its statement-budget charges:
    /// derivatives are transient solver stages, not retained outputs.
    fn od2_invoke(
        &mut self,
        c: &Callable,
        args: Vec<Value>,
        nout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        let evaluated = self.evaluated_elements;
        let result = (|| match c {
            Callable::Handle(h) => self.call_handle(h, args, nout, None, out),
            Callable::Name(n) => {
                let Some(resolved) = self.resolve_function(n)? else {
                    return Err(format!("Undefined function or variable '{n}'."));
                };
                self.invoke_function(resolved, args, nout, None, out)
            }
        })();
        self.evaluated_elements = evaluated;
        result
    }

    fn od2_vector(&self, v: Value) -> Result<Vec<f64>> {
        match v {
            Value::Void => Err("The function returned no value.".into()),
            Value::Complex(_) => Err("Complex values are not supported by this solver yet.".into()),
            other => Ok(other.matrix(&self.cancel)?.data().to_vec()),
        }
    }

    fn od2_derivative(&self, values: Vec<Value>, f: &Callable, neq: usize) -> Result<Vec<f64>> {
        let value = values.into_iter().next().unwrap_or(Value::Void);
        if matches!(value, Value::Void) {
            return Err(format!("{} must return a column vector.", f.diagnostic()));
        }
        let m = value.matrix(&self.cancel)?;
        if m.cols() > 1 {
            return Err(format!("{} must return a column vector.", f.diagnostic()));
        }
        if m.rows() != neq {
            let name = f.diagnostic();
            return Err(format!(
                "{name} returns a vector of length {}, but the length of initial conditions vector is {neq}. The vector returned by {name} and the initial conditions vector must have the same number of elements.",
                m.len()
            ));
        }
        Ok(m.data().to_vec())
    }

    fn od2_square(&self, values: Vec<Value>, neq: usize) -> Result<Vec<f64>> {
        let value = values.into_iter().next().unwrap_or(Value::Void);
        let m = value.matrix(&self.cancel)?;
        if m.rows() != neq || m.cols() != neq {
            return Err("Index exceeds matrix dimensions.".into());
        }
        Ok(m.data().to_vec())
    }

    fn od2_event_values(
        &mut self,
        ev: &Callable,
        args: Vec<Value>,
        out: &mut Execution,
    ) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        let values = self.od2_invoke(ev, args, 3, out)?;
        if values.len() != 3 {
            return Err("Event function must return three outputs.".into());
        }
        let mut vectors = values.into_iter().map(|value| self.od2_vector(value));
        let v = vectors.next().unwrap()?;
        let terminal = vectors.next().unwrap()?;
        let direction = vectors.next().unwrap()?;
        num::validate_events(&v, &terminal, &direction, v.len())?;
        Ok((v, terminal, direction))
    }

    fn od2_callable(&self, value: &Value) -> Option<Callable> {
        match value {
            Value::Container(Array::Handle(h)) => Some(Callable::Handle(h.clone())),
            Value::Text(s) if !s.is_empty() => Some(Callable::Name(s.clone())),
            _ => None,
        }
    }

    fn od2_is_empty(value: &Value) -> bool {
        match value {
            Value::Matrix(m) => m.is_empty(),
            Value::Logical(m) => m.is_empty(),
            Value::Complex(m) => m.is_empty(),
            Value::Container(a) => a.is_empty(),
            Value::Text(s) | Value::QuotedString(s) => s.is_empty(),
            Value::Void => true,
        }
    }

    fn od2_doubles(&self, value: &Value) -> Result<Vec<f64>> {
        match value {
            Value::Matrix(m) => Ok(m.data().to_vec()),
            Value::Logical(_) => Err("Invalid type for argument Y. Type should be double.".into()),
            Value::Complex(_) => {
                Err("Complex tspan or initial values are not supported yet".into())
            }
            Value::Container(Array::Typed(_)) => {
                Ok(value.clone().matrix(&self.cancel)?.data().to_vec())
            }
            _ => Err("Inputs must be floats, namely single or double.".into()),
        }
    }

    fn od2_options(&self, value: Option<&Value>, set: &str) -> Result<OptionSet> {
        match value {
            None => Ok(OptionSet { set: None }),
            Some(v) if Self::od2_is_empty(v) => Ok(OptionSet { set: None }),
            Some(Value::Container(Array::Struct(s))) if s.len() == 1 => Ok(OptionSet {
                set: Some(s.clone()),
            }),
            Some(_) => Err(format!(
                "Options must be [] or a structure created with {set}."
            )),
        }
    }

    fn od2_scalar(&self, a: &Array, name: &str) -> Result<f64> {
        match a {
            Array::Numeric(m) if m.len() == 1 => Ok(m.data()[0]),
            _ => Err(format!("{name} must be a real numeric scalar")),
        }
    }

    fn od2_text(&self, a: &Array) -> Result<String> {
        match a {
            Array::Character(c) => c.to_utf8(&self.cancel),
            _ => Err("Expected a character option value".into()),
        }
    }

    fn od2_numbers(&self, a: &Array) -> Result<Vec<f64>> {
        match a {
            Array::Numeric(m) => Ok(m.data().to_vec()),
            Array::Logical(_) | Array::Typed(_) => {
                Ok(Value::from(a.clone()).matrix(&self.cancel)?.data().to_vec())
            }
            _ => Err("Expected a numeric option value".into()),
        }
    }

    fn od2_struct(&self, names: &[&str], values: Vec<Array>) -> Result<Value> {
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names.iter().map(|n| (*n).to_string()).collect(),
            values.into_iter().map(Arc::new).collect(),
            &self.cancel,
        )?)))
    }

    fn od2_emit_lines(&mut self, out: &mut Execution, lines: Vec<String>) -> Result<()> {
        for line in lines {
            self.emit(out, line)?;
        }
        Ok(())
    }

    /// Tolerances and step bounds shared by the ODE solvers.
    #[allow(clippy::type_complexity)]
    fn od2_tolerances(
        &self,
        opts: &OptionSet,
        neq: usize,
        t0: f64,
        tfinal: f64,
        f: &Callable,
        out: &mut Vec<String>,
    ) -> Result<(f64, Vec<f64>, bool, Vec<f64>, f64, Option<f64>)> {
        let mut rtol = match opts.get("RelTol") {
            None => 1e-3,
            Some(value) => self
                .od2_scalar(&value, "RelTol")
                .map_err(|_| "RelTol must be a positive scalar.".to_string())?,
        };
        if !rtol.is_finite() || rtol <= 0. {
            return Err("RelTol must be a positive scalar.".into());
        }
        if rtol < 100. * f64::EPSILON {
            rtol = 100. * f64::EPSILON;
            out.push(format!(
                "Warning: RelTol has been increased to {}.",
                fmt_g(rtol)
            ));
        }
        let atol = match opts.get("AbsTol") {
            None => vec![1e-6],
            Some(value) => self.od2_numbers(&value)?,
        };
        if atol.len() != 1 && atol.len() != neq {
            return Err(format!(
                "Solving {} requires a scalar AbsTol, or a vector AbsTol of length {neq}",
                f.diagnostic()
            ));
        }
        Self::od2_validate_atol(&atol, neq)?;
        let normcontrol = match opts.get("NormControl") {
            None => false,
            Some(value) => self.od2_text(&value)? == "on",
        };
        if normcontrol && atol.len() != 1 {
            return Err("AbsTol must be a scalar when NormControl is 'on'.".into());
        }
        let threshold = (0..neq)
            .map(|j| atol[if atol.len() == 1 { 0 } else { j }] / rtol)
            .collect();
        let span = (tfinal - t0).abs();
        let hmax = match opts.get("MaxStep") {
            None => 0.1 * span,
            Some(value) => {
                let h = span.min(self.od2_scalar(&value, "MaxStep")?.abs());
                if h <= 0. {
                    return Err("Option 'MaxStep' must be greater than zero.".into());
                }
                h
            }
        };
        let htry = match opts.get("InitialStep") {
            None => None,
            Some(value) => {
                let h = self.od2_scalar(&value, "InitialStep")?.abs();
                if h <= 0. {
                    return Err("Option 'InitialStep' must be greater than zero.".into());
                }
                Some(h)
            }
        };
        Ok((rtol, atol, normcontrol, threshold, hmax, htry))
    }

    fn od2_tspan(tspan: &[f64]) -> Result<(f64, f64)> {
        if tspan.len() < 2 {
            return Err("tspan must have at least two elements.".into());
        }
        if tspan.iter().any(|x| !x.is_finite()) {
            return Err("The entries in tspan must be finite.".into());
        }
        let first = tspan[0];
        let last = tspan[tspan.len() - 1];
        if first == last {
            return Err("The last entry in tspan must be different from the first entry.".into());
        }
        let direction = num::sign(last - first);
        if tspan
            .windows(2)
            .any(|pair| direction * (pair[1] - pair[0]) <= 0.)
        {
            return Err("The entries in tspan must strictly increase or decrease.".into());
        }
        Ok((first, last))
    }

    fn od2_unsupported(opts: &OptionSet, names: &[&str], solver: &str) -> Result<()> {
        for name in names {
            if opts.get(name).is_some() {
                return Err(format!("{solver} option '{name}' is not supported yet"));
            }
        }
        Ok(())
    }

    fn od2_stats_lines(stats: &[usize]) -> Vec<String> {
        let labels = [
            "successful steps",
            "failed attempts",
            "function evaluations",
            "partial derivatives",
            "LU decompositions",
            "solutions of linear systems",
        ];
        stats
            .iter()
            .zip(labels)
            .map(|(count, label)| format!("{count} {label}"))
            .collect()
    }

    fn od2_stats_struct(&self, stats: &[usize]) -> Result<Array> {
        let names = [
            "nsteps", "nfailed", "nfevals", "npds", "ndecomps", "nsolves",
        ];
        Ok(Array::Struct(StructArray::from_slots(
            1,
            1,
            names
                .iter()
                .take(stats.len())
                .map(|s| s.to_string())
                .collect(),
            stats.iter().map(|count| num(*count as f64)).collect(),
            &self.cancel,
        )?))
    }

    fn od2_extdata(
        &self,
        f: &Callable,
        options: Option<&Value>,
        extras: &[Value],
        ypfinal: Option<Vec<f64>>,
    ) -> Result<Array> {
        let opts = match options {
            Some(v) => v.clone().array(&self.cancel)?,
            None => empty(),
        };
        let varargin = Array::Cell(CellArray::from_shared(
            1,
            extras.len(),
            extras
                .iter()
                .map(|v| v.clone().array(&self.cancel).map(Arc::new))
                .collect::<Result<_>>()?,
            &self.cancel,
        )?);
        let mut names = vec!["odefun", "options", "varargin"];
        let mut values = vec![f.value(), opts, varargin];
        if let Some(yp) = ypfinal {
            names.push("ypfinal");
            values.push(column(yp)?);
        }
        match self.od2_struct(&names, values)? {
            Value::Container(a) => Ok(a),
            _ => unreachable!(),
        }
    }

    /// ode113, ode15s, ode23s, ode23t and ode23tb.
    fn od2_ivp(
        &mut self,
        name: &str,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.is_empty() {
            let shown = if name == "ode15s" {
                "ODE15s".to_string()
            } else {
                upper(name)
            };
            return Err(format!("Not enough input arguments.  See {shown}."));
        }
        let args = self.text_arguments(args)?;
        let Some(f) = self.od2_callable(&args[0]) else {
            return Err(format!(
                "{name} expects a derivative function handle or function name as text"
            ));
        };
        let handle = matches!(f, Callable::Handle(_));
        if args.len() < 3 || Self::od2_is_empty(&args[1]) || Self::od2_is_empty(&args[2]) {
            return Err(format!(
                "When the first argument to {name} is a function handle, the tspan and y0 arguments must be supplied."
            ));
        }
        let tspan = self.od2_doubles(&args[1])?;
        if tspan.len() < 2 {
            return Err(format!(
                "When the first argument to {name} is a function handle, the tspan argument must have at least two elements."
            ));
        }
        let y0 = self.od2_doubles(&args[2])?;
        let neq = y0.len();
        if neq == 0 || y0.iter().any(|v| !v.is_finite()) {
            return Err("Initial conditions must be nonempty and finite.".into());
        }
        let options_value = args.get(3).cloned();
        let opts = self.od2_options(options_value.as_ref(), "ODESET")?;
        if name != "ode113" || opts.get("Mass").is_some() {
            Matrix::checked_len(neq, neq)?;
        }
        let extras: Vec<Value> = args.iter().skip(4).cloned().collect();
        let (t0, tfinal) = Self::od2_tspan(&tspan)?;
        Self::od2_unsupported(&opts, &["OutputFcn", "OutputSel", "NonNegative"], name)?;
        let mut warnings = Vec::new();
        let (rtol, atol, normcontrol, threshold, hmax, htry) =
            self.od2_tolerances(&opts, neq, t0, tfinal, &f, &mut warnings)?;
        let jac_opt = opts.get("Jacobian");
        let jac = match &jac_opt {
            None => JacKind::Numeric,
            Some(a) => match &**a {
                Array::Numeric(m) => {
                    if m.rows() != neq || m.cols() != neq {
                        return Err("Index exceeds matrix dimensions.".into());
                    }
                    JacKind::Matrix(m.data().to_vec())
                }
                Array::Handle(_) => JacKind::Function,
                _ => {
                    return Err("The Jacobian option must be a matrix or a function handle.".into());
                }
            },
        };
        let jac_fun = match &jac_opt {
            Some(a) => match &**a {
                Array::Handle(h) => Some(Callable::Handle(h.clone())),
                _ => None,
            },
            None => None,
        };
        let text_opt = |n: &str, d: &str| -> Result<String> {
            Ok(match opts.get(n) {
                None => d.into(),
                Some(v) => self.od2_text(&v)?.to_lowercase(),
            })
        };
        let jconstant = text_opt("JConstant", "off")? == "on";
        let stats_on = text_opt("Stats", "off")? == "on";
        let bdf = text_opt("BDF", "off")? == "on";
        let mass_singular = text_opt("MassSingular", "maybe")?;
        let mstate = text_opt("MStateDependence", "weak")?;
        let maxorder = match opts.get("MaxOrder") {
            None => 5,
            Some(v) => {
                let k = self.od2_scalar(&v, "MaxOrder")?;
                if !(1. ..=5.).contains(&k) || k.fract() != 0. {
                    return Err("MaxOrder must be 1, 2, 3, 4, or 5.".into());
                }
                k as usize
            }
        };
        let refine = match opts.get("Refine") {
            None => 1.,
            Some(v) => self.od2_scalar(&v, "Refine")?.max(1.),
        };
        if refine.fract() != 0. || refine > 1e6 {
            return Err("Refine must be a positive integer of at most 1000000".into());
        }
        let initial_slope = match opts.get("InitialSlope") {
            None => None,
            Some(v) => Some(self.od2_numbers(&v)?),
        };
        if initial_slope
            .as_ref()
            .is_some_and(|v| v.len() != neq || v.iter().any(|v| !v.is_finite()))
        {
            return Err(format!(
                "InitialSlope must be a finite vector of length {neq}."
            ));
        }
        let (mass, mass_fun) = match opts.get("Mass") {
            None => (MassKind::None, None),
            Some(a) => match &*a {
                Array::Numeric(m) => {
                    if m.rows() != neq || m.cols() != neq {
                        return Err("Index exceeds matrix dimensions.".into());
                    }
                    (MassKind::Matrix(m.data().to_vec()), None)
                }
                Array::Handle(h) => {
                    let state = mstate != "none";
                    (
                        MassKind::Function { state },
                        Some((Callable::Handle(h.clone()), state)),
                    )
                }
                _ => return Err("The Mass option must be a matrix or a function handle.".into()),
            },
        };
        let events_fun = match opts.get("Events") {
            None => None,
            Some(a) => match &*a {
                Array::Handle(h) => Some(Callable::Handle(h.clone())),
                Array::Character(c) => Some(Callable::Name(c.to_utf8(&self.cancel)?)),
                _ => return Err("The Events option must be a function handle.".into()),
            },
        };
        let sol_out = count == 1 && handle;
        let at = if sol_out {
            OutAt::Sol
        } else if tspan.len() > 2 {
            OutAt::Requested
        } else if refine > 1. {
            OutAt::Refined(refine as usize)
        } else {
            OutAt::Steps
        };
        let o = ivp::Opts {
            rtol,
            atol: atol.clone(),
            normcontrol,
            threshold,
            hmax,
            htry,
            htspan: (tspan[1] - tspan[0]).abs(),
            maxorder,
            bdf,
            jac,
            jconstant,
            mass,
            mass_singular,
            initial_slope,
            events: events_fun.is_some(),
        };
        let have_events = events_fun.is_some();
        let result = {
            let mut model = Ivp {
                e: self,
                out,
                f: f.clone(),
                extras: extras.clone(),
                jac: jac_fun,
                mass: mass_fun,
                events: events_fun.clone(),
                neq,
            };
            let f0 = model.f(t0, &y0)?;
            let valt = match &events_fun {
                Some(ev) => {
                    // Event setup evaluates the event function with one output first.
                    let args = model.args(vec![tv(t0), cv(&y0)]);
                    let v = model.e.od2_invoke(ev, args, 1, model.out)?;
                    Some(
                        model
                            .e
                            .od2_vector(v.into_iter().next().unwrap_or(Value::Void))?,
                    )
                }
                None => None,
            };
            let m0 = match &o.mass {
                MassKind::Function { .. } => Some(model.mass(t0, &y0)?),
                _ => None,
            };
            let setup = Setup {
                tspan: &tspan,
                y0: &y0,
                f0,
                valt,
                m0,
                at,
            };
            match name {
                "ode113" => ivp::ode113(&mut model, setup, &o)?,
                "ode15s" => ivp::ode15s(&mut model, setup, &o)?,
                "ode23s" => ivp::ode23s(&mut model, setup, &o)?,
                "ode23t" => ivp::ode23t(&mut model, setup, &o)?,
                _ => ivp::ode23tb(&mut model, setup, &o)?,
            }
        };
        warnings.extend(result.warnings.iter().cloned());
        self.od2_emit_lines(out, warnings)?;
        let full = name != "ode113";
        let stats: Vec<usize> = if full {
            result.stats.to_vec()
        } else {
            result.stats[..3].to_vec()
        };
        if stats_on {
            self.od2_emit_lines(out, Self::od2_stats_lines(&stats))?;
        }
        self.check_cancel()?;
        if sol_out {
            let n = result.t.len();
            let mut names = vec!["solver", "extdata", "x", "y"];
            let mut values = vec![
                text(name)?,
                self.od2_extdata(&f, options_value.as_ref(), &extras, None)?,
                row(result.t.clone())?,
                columns(&result.y, neq)?,
            ];
            if have_events {
                names.extend(["xe", "ye", "ie"]);
                values.extend(if result.te.is_empty() {
                    vec![empty(), empty(), empty()]
                } else {
                    vec![
                        row(result.te.clone())?,
                        columns(&result.ye, neq)?,
                        row(result.ie.iter().map(|i| *i as f64).collect())?,
                    ]
                });
            }
            names.push("stats");
            values.push(self.od2_stats_struct(&stats)?);
            names.push("idata");
            values.push(if result.data_dropped {
                Array::Struct(StructArray::scalar_empty(&self.cancel)?)
            } else {
                self.od2_idata(&result.data, n, neq)?
            });
            return Ok(vec![self.od2_struct(&names, values)?]);
        }
        let mut outputs = vec![Value::from(column(result.t.clone())?)];
        if count >= 2 {
            outputs.push(Value::from(rows_of(&result.y, neq)?));
        }
        if count >= 3 {
            if !have_events {
                return Err("Too many output arguments.".into());
            }
            if result.te.is_empty() {
                outputs.extend((0..3).map(|_| Value::from(empty())));
            } else {
                outputs.push(Value::from(column(result.te.clone())?));
                outputs.push(Value::from(rows_of(&result.ye, neq)?));
                outputs.push(Value::from(column(
                    result.ie.iter().map(|i| *i as f64).collect(),
                )?));
            }
            outputs.truncate(count);
        }
        Ok(outputs)
    }

    /// sol.idata for the step data of one solver; the first point carries zeros.
    fn od2_idata(&self, data: &[StepData], n: usize, neq: usize) -> Result<Array> {
        let nonneg = empty();
        // Interpolation data is bounded like any array: past the element limit
        // it is left out and deval reports the solution unusable.
        if n.saturating_mul(neq.max(1)).saturating_mul(16) > crate::matrix::MAX_ELEMENTS {
            return Ok(Array::Struct(StructArray::scalar_empty(&self.cancel)?));
        }
        let (names, values): (Vec<&str>, Vec<Array>) = match data.first() {
            Some(StepData::Abm { .. }) | None => {
                let mut kvec = vec![0.];
                let mut phi = vec![0.; neq * 14];
                let mut psi = vec![0.; 12];
                for d in data {
                    if let StepData::Abm { k, phi: p, psi: s } = d {
                        kvec.push(*k as f64);
                        for column in &p[1..=14] {
                            phi.extend_from_slice(column);
                        }
                        psi.extend_from_slice(&s[1..=12]);
                    }
                }
                let _ = n;
                (
                    vec!["kvec", "phi3d", "psi2d", "idxNonNegative"],
                    vec![
                        row(kvec)?,
                        Array::Numeric(Matrix::from_column_vec(neq, phi.len() / neq.max(1), phi)?),
                        Array::Numeric(Matrix::from_column_vec(12, psi.len() / 12, psi)?),
                        nonneg,
                    ],
                )
            }
            Some(StepData::Ndf { dif, .. }) => {
                let width = dif.len() - 1;
                let mut kvec = vec![0.];
                let mut all = vec![0.; neq * width];
                for d in data {
                    if let StepData::Ndf { k, dif } = d {
                        kvec.push(*k as f64);
                        for c in dif.iter().skip(1) {
                            all.extend_from_slice(c);
                        }
                    }
                }
                (
                    vec!["kvec", "dif3d", "idxNonNegative"],
                    vec![
                        row(kvec)?,
                        Array::Numeric(Matrix::from_column_vec(neq, all.len() / neq.max(1), all)?),
                        nonneg,
                    ],
                )
            }
            Some(StepData::Ros { .. }) => {
                let mut k1 = vec![vec![0.; neq]];
                let mut k2 = vec![vec![0.; neq]];
                for d in data {
                    if let StepData::Ros { k1: a, k2: b } = d {
                        k1.push(a.clone());
                        k2.push(b.clone());
                    }
                }
                (
                    vec!["k1", "k2"],
                    vec![columns(&k1, neq)?, columns(&k2, neq)?],
                )
            }
            Some(StepData::Tr { .. }) => {
                let mut z = vec![vec![0.; neq]];
                let mut zn = vec![vec![0.; neq]];
                for d in data {
                    if let StepData::Tr { z: a, znew: b } = d {
                        z.push(a.clone());
                        zn.push(b.clone());
                    }
                }
                (
                    vec!["z", "znew", "idxNonNegative"],
                    vec![columns(&z, neq)?, columns(&zn, neq)?, nonneg],
                )
            }
            Some(StepData::TrBdf { .. }) => {
                let mut t2 = vec![0.];
                let mut y2 = vec![vec![0.; neq]];
                for d in data {
                    if let StepData::TrBdf { t2: a, y2: b } = d {
                        t2.push(*a);
                        y2.push(b.clone());
                    }
                }
                (
                    vec!["t2", "y2", "idxNonNegative"],
                    vec![row(t2)?, columns(&y2, neq)?, nonneg],
                )
            }
        };
        match self.od2_struct(&names, values)? {
            Value::Container(a) => Ok(a),
            _ => unreachable!(),
        }
    }

    fn od2_ode15i(
        &mut self,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.len() < 4 {
            return Err("Not enough input arguments.  See ODE15I.".into());
        }
        let args = self.text_arguments(args)?;
        let Some(f) = self.od2_callable(&args[0]) else {
            return Err("ode15i expects a function handle or function name as text".into());
        };
        let handle = matches!(f, Callable::Handle(_));
        if Self::od2_is_empty(&args[1]) || Self::od2_is_empty(&args[2]) {
            return Err("When the first argument to ode15i is a function handle, the tspan and y0 arguments must be supplied.".into());
        }
        let tspan = self.od2_doubles(&args[1])?;
        if tspan.len() < 2 {
            return Err("When the first argument to ode15i is a function handle, the tspan argument must have at least two elements.".into());
        }
        let y0 = self.od2_doubles(&args[2])?;
        let yp0 = self.od2_doubles(&args[3])?;
        let neq = y0.len();
        if neq == 0 || y0.iter().chain(&yp0).any(|v| !v.is_finite()) {
            return Err("Initial conditions must be nonempty and finite.".into());
        }
        if yp0.len() != neq {
            return Err("y0 and yp0 must have the same number of elements.".into());
        }
        Matrix::checked_len(neq, neq)?;
        let options_value = args.get(4).cloned();
        let opts = self.od2_options(options_value.as_ref(), "ODESET")?;
        let extras: Vec<Value> = args.iter().skip(5).cloned().collect();
        let (t0, tfinal) = Self::od2_tspan(&tspan)?;
        Self::od2_unsupported(
            &opts,
            &[
                "OutputFcn",
                "OutputSel",
                "Events",
                "Jacobian",
                "NonNegative",
            ],
            "ode15i",
        )?;
        let mut warnings = Vec::new();
        let (rtol, atol, normcontrol, threshold, hmax, htry) =
            self.od2_tolerances(&opts, neq, t0, tfinal, &f, &mut warnings)?;
        if normcontrol {
            return Err("ode15i option 'NormControl' is not supported yet".into());
        }
        let stats_on = matches!(opts.get("Stats"), Some(v) if self.od2_text(&v)? == "on");
        let maxorder = match opts.get("MaxOrder") {
            None => 5,
            Some(v) => self.od2_scalar(&v, "MaxOrder")?.clamp(1., 5.) as usize,
        };
        let refine = match opts.get("Refine") {
            None => 1.,
            Some(v) => self.od2_scalar(&v, "Refine")?.max(1.),
        };
        if refine.fract() != 0. || refine > 1e6 {
            return Err("Refine must be a positive integer of at most 1000000".into());
        }
        let sol_out = count == 1 && handle;
        let at = if sol_out {
            OutAt::Sol
        } else if tspan.len() > 2 {
            OutAt::Requested
        } else if refine > 1. {
            OutAt::Refined(refine as usize)
        } else {
            OutAt::Steps
        };
        let io = implicit::IOpts {
            rtol,
            threshold,
            atol,
            hmax,
            htry,
            htspan: (tspan[1] - tspan[0]).abs(),
            maxorder,
        };
        let result = {
            let mut model = ImplicitIvp {
                e: self,
                out,
                f: f.clone(),
                extras: extras.clone(),
                neq,
            };
            implicit::Implicit::f(&mut model, t0, &y0, &yp0)?;
            implicit::ode15i(&mut model, &tspan, &y0, &yp0, &io, at)?
        };
        warnings.extend(result.warnings.iter().cloned());
        self.od2_emit_lines(out, warnings)?;
        if stats_on {
            self.od2_emit_lines(out, Self::od2_stats_lines(&result.stats))?;
        }
        if sol_out {
            let kvec = row(result.kvec.iter().map(|k| *k as f64).collect())?;
            let idata = self
                .od2_struct(&["kvec"], vec![kvec])?
                .array(&self.cancel)?;
            let values = vec![
                text("ode15i")?,
                self.od2_extdata(
                    &f,
                    options_value.as_ref(),
                    &extras,
                    Some(result.ypfinal.clone()),
                )?,
                row(result.t.clone())?,
                columns(&result.y, neq)?,
                self.od2_stats_struct(&result.stats)?,
                idata,
            ];
            return Ok(vec![self.od2_struct(
                &["solver", "extdata", "x", "y", "stats", "idata"],
                values,
            )?]);
        }
        let mut outputs = vec![Value::from(column(result.t.clone())?)];
        if count >= 2 {
            outputs.push(Value::from(rows_of(&result.y, neq)?));
        }
        if count > 2 {
            return Err("Too many output arguments.".into());
        }
        Ok(outputs)
    }

    fn od2_decic(
        &mut self,
        args: Vec<Value>,
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.len() < 6 {
            return Err("Not enough input arguments.".into());
        }
        let args = self.text_arguments(args)?;
        let Some(f) = self.od2_callable(&args[0]) else {
            return Err("decic expects a function handle or function name as text".into());
        };
        let t0 = self.od2_doubles(&args[1])?.first().copied().unwrap_or(0.);
        let y0 = self.od2_doubles(&args[2])?;
        if y0.is_empty() || !t0.is_finite() || y0.iter().any(|v| !v.is_finite()) {
            return Err("Initial conditions must be nonempty and finite.".into());
        }
        for a in &args[3..6] {
            if self.od2_doubles(a)?.iter().any(|v| !v.is_finite()) {
                return Err(
                    "Initial conditions and fixed-component vectors must be finite.".into(),
                );
            }
        }
        let fixed_y: Vec<bool> = self
            .od2_doubles(&args[3])?
            .iter()
            .map(|v| *v != 0.)
            .collect();
        let yp0 = self.od2_doubles(&args[4])?;
        let fixed_yp: Vec<bool> = self
            .od2_doubles(&args[5])?
            .iter()
            .map(|v| *v != 0.)
            .collect();
        let neq = y0.len();
        if yp0.len() != neq || fixed_y.len() != neq || fixed_yp.len() != neq {
            return Err(
                "The vectors y0, fixed_y0, yp0 and fixed_yp0 must have the same length.".into(),
            );
        }
        if fixed_y.iter().chain(&fixed_yp).filter(|b| **b).count() > neq {
            return Err(format!(
                "Cannot specify more than {neq} components of y0 and yp0."
            ));
        }
        let opts = self.od2_options(args.get(6), "ODESET")?;
        let extras: Vec<Value> = args.iter().skip(7).cloned().collect();
        let rtol = match opts.get("RelTol") {
            None => 1e-3,
            Some(v) => self.od2_scalar(&v, "RelTol")?,
        };
        let atol = match opts.get("AbsTol") {
            None => vec![1e-6],
            Some(v) => self.od2_numbers(&v)?,
        };
        Self::od2_validate_atol(&atol, neq)?;
        if !rtol.is_finite() || rtol <= 0. {
            return Err("RelTol must be a positive scalar.".into());
        }
        Matrix::checked_len(neq, neq)?;
        Matrix::checked_len(
            neq,
            fixed_y.iter().chain(&fixed_yp).filter(|v| !**v).count(),
        )?;
        let (y, yp, resnrm) = {
            let mut model = ImplicitIvp {
                e: self,
                out,
                f,
                extras,
                neq,
            };
            implicit::decic(&mut model, t0, &y0, &fixed_y, &yp0, &fixed_yp, rtol, &atol)?
        };
        let mut outputs = vec![Value::from(column(y)?)];
        if count >= 2 {
            outputs.push(Value::from(column(yp)?));
        }
        if count >= 3 {
            outputs.push(Value::Matrix(Matrix::scalar(resnrm)));
        }
        Ok(outputs)
    }

    /// Fields of a solution structure.
    fn od2_validate_atol(atol: &[f64], neq: usize) -> Result<()> {
        if atol.len() != 1 && atol.len() != neq {
            return Err(format!(
                "AbsTol must be a scalar or a vector of length {neq}."
            ));
        }
        if atol.is_empty() || atol.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return Err("AbsTol must be positive.".into());
        }
        Ok(())
    }

    fn od2_field(sol: &StructArray, name: &str) -> Option<Arc<Array>> {
        let i = sol.field_index(name)?;
        sol.get(i, 0).ok().map(Arc::clone)
    }

    fn od2_matrix(a: &Array) -> Option<Matrix> {
        match a {
            Array::Numeric(m) if !m.is_empty() => Some(m.clone()),
            _ => None,
        }
    }

    /// Validate editable solution arrays before any interpolation indexing.
    fn od2_validate_mesh(x: &Matrix, y: &Matrix) -> Result<()> {
        let t = x.data();
        if t.len() < 2
            || (x.rows() != 1 && x.cols() != 1)
            || y.cols() != t.len()
            || y.rows() == 0
            || t.iter().chain(y.data()).any(|v| !v.is_finite())
        {
            return Err(" must be a structure returned by a differential equation solver.".into());
        }
        let dir = (t[t.len() - 1] - t[0]).signum();
        if dir == 0. || t.windows(2).any(|w| dir * (w[1] - w[0]) <= 0.) {
            return Err("The solution mesh must be strictly monotone and finite.".into());
        }
        Ok(())
    }

    /// Follow the saved history chain for times preceding a continued DDE mesh.
    fn od2_history_value(
        &mut self,
        mut sol: StructArray,
        t: f64,
        extras: &[Value],
        out: &mut Execution,
    ) -> Result<Vec<f64>> {
        for _ in 0..64 {
            self.check_cancel()?;
            let x = Self::od2_field(&sol, "x")
                .and_then(|a| Self::od2_matrix(&a))
                .ok_or("The history structure needs fields x, y and yp.")?;
            let y = Self::od2_field(&sol, "y")
                .and_then(|a| Self::od2_matrix(&a))
                .ok_or("The history structure needs fields x, y and yp.")?;
            Self::od2_validate_mesh(&x, &y)?;
            if x.data()[0] > x.data()[x.len() - 1] {
                return Err("DDE history mesh must increase.".into());
            }
            if t >= x.data()[0] {
                let yp = Self::od2_field(&sol, "yp")
                    .and_then(|a| Self::od2_matrix(&a))
                    .ok_or("The history structure needs fields x, y and yp.")?;
                if yp.rows() != y.rows()
                    || yp.cols() != y.cols()
                    || yp.data().iter().any(|v| !v.is_finite())
                {
                    return Err("The history derivative array must match y.".into());
                }
                let b = x.data().partition_point(|s| *s <= t).clamp(1, x.len() - 1);
                let r = y.rows();
                return Ok(bvp::hermite(
                    t,
                    x.data()[b - 1],
                    &y.data()[(b - 1) * r..b * r],
                    &yp.data()[(b - 1) * r..b * r],
                    x.data()[b],
                    &y.data()[b * r..(b + 1) * r],
                    &yp.data()[b * r..(b + 1) * r],
                )
                .0);
            }
            let history = Self::od2_field(&sol, "history")
                .ok_or("The history structure needs its saved history.")?;
            match &*history {
                Array::Struct(s) => sol = s.clone(),
                Array::Handle(h) => {
                    let mut args = vec![tv(t)];
                    args.extend(extras.iter().cloned());
                    let values = self.od2_invoke(&Callable::Handle(h.clone()), args, 1, out)?;
                    return self.od2_vector(values.into_iter().next().unwrap_or(Value::Void));
                }
                _ => return self.od2_numbers(&history),
            }
        }
        Err("DDE history nesting exceeds the limit of 64 solutions.".into())
    }

    fn od2_deval(&mut self, args: Vec<Value>, count: usize) -> Result<Vec<Value>> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let (sol, xint) = match &args[0] {
            Value::Container(Array::Struct(_)) => (&args[0], &args[1]),
            _ => (&args[1], &args[0]),
        };
        let not_sol =
            || " must be a structure returned by a differential equation solver.".to_string();
        let Value::Container(Array::Struct(sol)) = sol else {
            return Err(not_sol());
        };
        if sol.len() != 1 {
            return Err(not_sol());
        }
        let (Some(x), Some(y)) = (Self::od2_field(sol, "x"), Self::od2_field(sol, "y")) else {
            return Err(not_sol());
        };
        let (Some(x), Some(y)) = (Self::od2_matrix(&x), Self::od2_matrix(&y)) else {
            return Err(not_sol());
        };
        let neq = y.rows();
        let idx: Vec<usize> = match args.get(2) {
            None => (1..=neq).collect(),
            Some(v) => {
                let v = self.od2_doubles(v)?;
                if v.iter().any(|i| *i < 0. || *i > neq as f64) {
                    return Err("Incorrect solution component requested in .".into());
                }
                if v.iter().any(|i| *i < 1. || i.fract() != 0.) {
                    return Err("Index exceeds matrix dimensions.".into());
                }
                v.iter().map(|i| *i as usize).collect()
            }
        };
        let Some(solver) = Self::od2_field(sol, "solver") else {
            return Err(not_sol());
        };
        let solver = self.od2_text(&solver).map_err(|_| not_sol())?;
        let tint = self.od2_doubles(xint)?;
        if tint.is_empty() {
            return Err("Index exceeds matrix dimensions.".into());
        }
        let output_len = Matrix::checked_len(idx.len(), tint.len())?;
        let t = x.data();
        let n = t.len();
        Self::od2_validate_mesh(&x, &y)?;
        let tdir = (t[n - 1] - t[0]).signum();
        let (lo, hi) = tint
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                (a.min(tdir * v), b.max(tdir * v))
            });
        if tint.iter().any(|v| v.is_nan()) || lo < tdir * t[0] || hi > tdir * t[n - 1] {
            return Err(format!(
                "Attempting to evaluate the solution outside the interval [{}, {}] where it is defined.",
                ivp::format_e(t[0]),
                ivp::format_e(t[n - 1])
            ));
        }
        let ycol = |i: usize| y.data()[i * neq..(i + 1) * neq].to_vec();
        let idata = Self::od2_field(sol, "idata").and_then(|a| match &*a {
            Array::Struct(s) => Some(s.clone()),
            _ => None,
        });
        if idata.as_ref().is_some_and(|s| s.field_names().is_empty()) {
            return Err(
                "The solution has no interpolation data (sol.idata exceeded the element limit)."
                    .into(),
            );
        }
        let idata_mat = |name: &str| -> Option<Matrix> {
            idata
                .as_ref()
                .and_then(|s| Self::od2_field(s, name))
                // Empty f3d denotes intentionally dropped interpolation data;
                // keep it here so the solver-specific diagnostic is reachable.
                .and_then(|a| match &*a {
                    Array::Numeric(m) if name == "f3d" || !m.is_empty() => Some(m.clone()),
                    _ => None,
                })
        };
        let yp_mesh = Self::od2_field(sol, "yp").and_then(|a| Self::od2_matrix(&a));
        // idata is user-editable. Check every dimension and order once before
        // slicing; a valid matrix alone does not imply a valid interpolant.
        let checked = |name: &str, rows: usize, cols: usize| -> Result<Matrix> {
            let m = idata_mat(name).ok_or_else(not_sol)?;
            if m.rows() != rows || m.cols() != cols || m.data().iter().any(|v| !v.is_finite()) {
                return Err(format!("Invalid solution interpolation data: {name}."));
            }
            Ok(m)
        };
        let orders = |max: usize| -> Result<Matrix> {
            let k = checked("kvec", 1, n)?;
            if k.data()
                .iter()
                .skip(1)
                .any(|v| *v < 1. || *v > max as f64 || v.fract() != 0.)
            {
                return Err("Invalid solution interpolation orders.".into());
            }
            Ok(k)
        };
        match solver.as_str() {
            "ode45" | "ode23" => {
                if idata_mat("f3d").is_some_and(|m| m.is_empty()) {
                    return Err("The solution has no interpolation data (sol.idata.f3d exceeded the element limit).".into());
                }
                checked("f3d", neq, n * if solver == "ode45" { 6 } else { 4 })?;
            }
            "ode113" => {
                checked("phi3d", neq, n * 14)?;
                let ps = checked("psi2d", 12, n)?;
                let ks = orders(12)?;
                for b in 1..n {
                    if ps.data()[b * 12..b * 12 + ks.data()[b] as usize].contains(&0.) {
                        return Err("Invalid solution interpolation data: psi2d.".into());
                    }
                }
            }
            "ode15s" => {
                let dif = idata_mat("dif3d").ok_or_else(not_sol)?;
                let width = dif.cols() / n;
                if !(3..=7).contains(&width) {
                    return Err(not_sol());
                }
                checked("dif3d", neq, n * width)?;
                orders(width - 2)?;
            }
            "ode23s" => {
                checked("k1", neq, n)?;
                checked("k2", neq, n)?;
            }
            "ode23t" => {
                checked("z", neq, n)?;
                checked("znew", neq, n)?;
            }
            "ode23tb" => {
                let mid = checked("t2", 1, n)?;
                checked("y2", neq, n)?;
                for b in 1..n {
                    if mid.data()[b] == t[b] || mid.data()[b] == t[b - 1] {
                        return Err("Invalid solution interpolation data: t2.".into());
                    }
                }
            }
            "ode15i" => {
                orders(5)?;
            }
            "bvp5c" => {
                checked("yp", neq, n)?;
                checked("ypmid", neq, 2 * (n - 1))?;
            }
            _ => {
                let yp = yp_mesh.as_ref().ok_or_else(not_sol)?;
                if yp.rows() != neq || yp.cols() != n || yp.data().iter().any(|v| !v.is_finite()) {
                    return Err(not_sol());
                }
            }
        }
        let mut ys = Vec::with_capacity(output_len);
        let mut yps = Vec::with_capacity(if count >= 2 { output_len } else { 0 });
        for (k, &ti) in tint.iter().enumerate() {
            if k % 64 == 0 {
                self.check_cancel()?;
            }
            // Right-open interval [t(i), t(i+1)); the last one is closed.
            let mut i = t.partition_point(|v| tdir * v <= tdir * ti);
            i = i.clamp(1, n - 1);
            let (a, b) = (i - 1, i);
            let h = t[b] - t[a];
            let (yv, ypv) = match solver.as_str() {
                "ode45" | "ode23" => {
                    let f3d = idata_mat("f3d").ok_or_else(not_sol)?;
                    let width = f3d.cols() / n;
                    if width == 0 {
                        return Err("The solution has no interpolation data (sol.idata.f3d exceeded the element limit).".into());
                    }
                    let st: Vec<Vec<f64>> = (0..width)
                        .map(|s| {
                            f3d.data()[(b * width + s) * neq..(b * width + s + 1) * neq].to_vec()
                        })
                        .collect();
                    let refs: Vec<&[f64]> = st.iter().map(|v| v.as_slice()).collect();
                    let bi45: [[f64; 4]; 6] = [
                        [1., -183. / 64., 37. / 12., -145. / 128.],
                        [0., 1500. / 371., -1000. / 159., 1000. / 371.],
                        [0., -125. / 32., 125. / 12., -375. / 64.],
                        [0., 9477. / 3392., -729. / 106., 25515. / 6784.],
                        [0., -11. / 7., 11. / 3., -55. / 28.],
                        [0., 3. / 2., -4., 5. / 2.],
                    ];
                    let bi23: [[f64; 4]; 4] = [
                        [1., -4. / 3., 5. / 9., 0.],
                        [0., 1., -2. / 3., 0.],
                        [0., 4. / 3., -8. / 9., 0.],
                        [0., -1., 1., 0.],
                    ];
                    let bi: &[[f64; 4]] = if solver == "ode45" { &bi45 } else { &bi23 };
                    interp::rk(bi, &refs, t[a], &ycol(a), h, ti)
                }
                "ode113" => {
                    let phi = idata_mat("phi3d").ok_or_else(not_sol)?;
                    let psi = idata_mat("psi2d").ok_or_else(not_sol)?;
                    let kvec = idata_mat("kvec").ok_or_else(not_sol)?;
                    let mut ph = vec![vec![0.; neq]];
                    for c in 0..14 {
                        let col = b * 14 + c;
                        ph.push(phi.data()[col * neq..(col + 1) * neq].to_vec());
                    }
                    let mut ps = vec![0.];
                    ps.extend_from_slice(&psi.data()[b * 12..(b + 1) * 12]);
                    ps.push(0.);
                    interp::abm(ti, t[b], &ycol(b), kvec.data()[b] as usize, &ph, &ps)
                }
                "ode15s" => {
                    let dif = idata_mat("dif3d").ok_or_else(not_sol)?;
                    let kvec = idata_mat("kvec").ok_or_else(not_sol)?;
                    let width = dif.cols() / n;
                    let mut d = vec![vec![0.; neq]];
                    for c in 0..width {
                        let col = b * width + c;
                        d.push(dif.data()[col * neq..(col + 1) * neq].to_vec());
                    }
                    interp::ndf(ti, t[b], &ycol(b), h, &d, kvec.data()[b] as usize)
                }
                "ode23s" => {
                    let k1 = idata_mat("k1").ok_or_else(not_sol)?;
                    let k2 = idata_mat("k2").ok_or_else(not_sol)?;
                    let c = |m: &Matrix| m.data()[b * neq..(b + 1) * neq].to_vec();
                    interp::ros(ti, t[a], &ycol(a), h, &c(&k1), &c(&k2))
                }
                "ode23t" => {
                    let z = idata_mat("z").ok_or_else(not_sol)?;
                    let zn = idata_mat("znew").ok_or_else(not_sol)?;
                    let c = |m: &Matrix| m.data()[b * neq..(b + 1) * neq].to_vec();
                    interp::tr(ti, t[a], &ycol(a), h, &ycol(b), &c(&z), &c(&zn))
                }
                "ode23tb" => {
                    let t2 = idata_mat("t2").ok_or_else(not_sol)?;
                    let y2 = idata_mat("y2").ok_or_else(not_sol)?;
                    interp::trbdf(
                        ti,
                        t[a],
                        &ycol(a),
                        t[b],
                        &ycol(b),
                        t2.data()[b],
                        &y2.data()[b * neq..(b + 1) * neq],
                    )
                }
                "ode15i" => {
                    let kvec = idata_mat("kvec").ok_or_else(not_sol)?;
                    let k = (kvec.data()[b] as usize).clamp(1, b);
                    let ts: Vec<f64> = (b - k..=b).map(|j| t[j]).collect();
                    let cols: Vec<Vec<f64>> = (b - k..=b).map(ycol).collect();
                    let refs: Vec<&[f64]> = cols.iter().map(|v| v.as_slice()).collect();
                    interp::poly(ti, &ts, &refs)
                }
                "bvp5c" => {
                    let yp = idata_mat("yp").ok_or_else(not_sol)?;
                    let ypm = idata_mat("ypmid").ok_or_else(not_sol)?;
                    let c = |m: &Matrix, j: usize| m.data()[j * neq..(j + 1) * neq].to_vec();
                    let (f0, f1) = (c(&yp, a), c(&yp, b));
                    let (f2, f3) = (c(&ypm, 2 * a), c(&ypm, 2 * a + 1));
                    bvp::lobatto_eval(ti, t[a], h, &ycol(a), [&f0, &f2, &f3, &f1])
                }
                _ => {
                    // bvp4c, dde23, ddesd and bvpinit-like structures with yp: cubic Hermite.
                    let yp = yp_mesh.clone().ok_or_else(not_sol)?;
                    let c = |j: usize| yp.data()[j * neq..(j + 1) * neq].to_vec();
                    if h == 0. {
                        (ycol(a), c(a))
                    } else {
                        bvp::hermite(ti, t[a], &ycol(a), &c(a), t[b], &ycol(b), &c(b))
                    }
                }
            };
            let yv = if ti == t[b] && !matches!(solver.as_str(), "ode113" | "ode15s") {
                ycol(b)
            } else if ti == t[a] {
                ycol(a)
            } else {
                yv
            };
            ys.extend(idx.iter().map(|i| yv[i - 1]));
            if count >= 2 {
                yps.extend(idx.iter().map(|i| ypv[i - 1]));
            }
        }
        let mut outputs = vec![Value::Matrix(Matrix::from_column_vec(
            idx.len(),
            tint.len(),
            ys,
        )?)];
        if count >= 2 {
            outputs.push(Value::Matrix(Matrix::from_column_vec(
                idx.len(),
                tint.len(),
                yps,
            )?));
        }
        Ok(outputs)
    }

    fn od2_odextend(&mut self, args: Vec<Value>, out: &mut Execution) -> Result<Value> {
        if args.len() < 3 {
            return Err("Not enough input arguments.  See ODEXTEND.".into());
        }
        let invalid =
            || "Input argument  must be a structure created by an ODE solver.".to_string();
        let Value::Container(Array::Struct(sol)) = &args[0] else {
            return Err(invalid());
        };
        let solver = Self::od2_field(sol, "solver").ok_or_else(invalid)?;
        let solver = self.od2_text(&solver).map_err(|_| invalid())?;
        if !SOLVERS.contains(&solver.as_str()) && solver != "ode45" && solver != "ode23" {
            return Err(invalid());
        }
        let extdata = Self::od2_field(sol, "extdata").ok_or_else(invalid)?;
        let Array::Struct(extdata) = &*extdata else {
            return Err(invalid());
        };
        let x = Self::od2_field(sol, "x")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or_else(invalid)?;
        let y = Self::od2_field(sol, "y")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or_else(invalid)?;
        let n = x.len();
        let neq = y.rows();
        Self::od2_validate_mesh(&x, &y)?;
        self.od2_deval(vec![args[0].clone(), tv(x.data()[n - 1])], 1)?;
        let odefun = if Self::od2_is_empty(&args[1]) {
            Value::from((*Self::od2_field(extdata, "odefun").ok_or_else(invalid)?).clone())
        } else {
            args[1].clone()
        };
        let tfinal = self
            .od2_doubles(&args[2])?
            .first()
            .copied()
            .ok_or("tfinal must be a scalar")?;
        if !tfinal.is_finite() {
            return Err("tfinal must be finite.".into());
        }
        let t0 = x.data()[n - 1];
        let tstart = x.data()[0];
        let tdir = (t0 - tstart).signum();
        if tdir * (tfinal - t0) <= 0. {
            self.emit(
                out,
                format!(
                    "Warning: Solution already available in [{}, {}]. Use DEVAL.",
                    fmt_g(tstart),
                    fmt_g(tfinal)
                ),
            )?;
            return Ok(args[0].clone());
        }
        let yinit = match args.get(3) {
            Some(v) if !Self::od2_is_empty(v) => v.clone(),
            _ => Value::from(column(y.data()[(n - 1) * neq..].to_vec())?),
        };
        let original = Value::from(
            (*Self::od2_field(extdata, "options").unwrap_or_else(|| Arc::new(empty()))).clone(),
        );
        let options = match args.get(4) {
            Some(v) if !Self::od2_is_empty(v) => self
                .ode_call("odeset", &[original, v.clone()], out)?
                .ok_or_else(invalid)?,
            _ => original,
        };
        let mut extras: Vec<Value> = args.iter().skip(5).cloned().collect();
        if args.len() <= 5
            && let Some(v) = Self::od2_field(extdata, "varargin")
            && let Array::Cell(c) = &*v
        {
            for i in 0..c.len() {
                extras.push(Value::from((**c.get_linear(i)?).clone()));
            }
        }
        let tspan = Value::from(row(vec![t0, tfinal])?);
        let mut call = vec![odefun, tspan, yinit];
        if solver == "ode15i" {
            let yp = Self::od2_field(extdata, "ypfinal").ok_or_else(invalid)?;
            call.push(Value::from((*yp).clone()));
        }
        call.push(options);
        call.extend(extras);
        let new = if solver == "ode45" || solver == "ode23" {
            self.ode_call(&solver, &call, out)?.ok_or_else(invalid)?
        } else {
            self.od2_dispatch(&solver, call, 1, out)?.remove(0)
        };
        let Value::Container(Array::Struct(new)) = new else {
            return Err(invalid());
        };
        let nx = Self::od2_field(&new, "x")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or_else(invalid)?;
        let ny = Self::od2_field(&new, "y")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or_else(invalid)?;
        let n2 = nx.len();
        // Merge: the extension's first point repeats the old last point.
        let join =
            |old: &Matrix, add: &Matrix, n1: usize, n2: usize, pad: bool| -> Result<Matrix> {
                let r = old.rows();
                if n1 == 0
                    || n2 == 0
                    || !old.cols().is_multiple_of(n1)
                    || !add.cols().is_multiple_of(n2)
                    || r != add.rows()
                {
                    return Err(invalid());
                }
                let w1 = old.cols() / n1;
                let w2 = add.cols() / n2;
                if w1 != w2 && !pad {
                    return Err(invalid());
                }
                let width = w1.max(w2);
                let cols = (n1 + n2 - 1) * width;
                let mut data = Vec::with_capacity(Matrix::checked_len(r, cols)?);
                for (m, w, start, end) in [(old, w1, 0, n1), (add, w2, 1, n2)] {
                    for b in start..end {
                        self.check_cancel()?;
                        data.extend_from_slice(&m.data()[b * w * r..(b + 1) * w * r]);
                        data.resize(data.len() + (width - w) * r, 0.);
                    }
                }
                Matrix::from_column_vec(r, cols, data)
            };
        let xs = join(&x, &nx, n, n2, false)?;
        let ys = join(&y, &ny, n, n2, false)?;
        let stats_old = Self::od2_field(sol, "stats");
        let stats_new = Self::od2_field(&new, "stats");
        let stats = match (stats_old.as_deref(), stats_new.as_deref()) {
            (Some(Array::Struct(a)), Some(Array::Struct(b))) => {
                let mut slots = Vec::new();
                for name in a.field_names() {
                    let va = Self::od2_field(a, name)
                        .and_then(|v| Self::od2_matrix(&v))
                        .map(|m| m.data()[0])
                        .unwrap_or(0.);
                    let vb = Self::od2_field(b, name)
                        .and_then(|v| Self::od2_matrix(&v))
                        .map(|m| m.data()[0])
                        .unwrap_or(0.);
                    slots.push(num(va + vb));
                }
                Array::Struct(StructArray::from_slots(
                    1,
                    1,
                    a.field_names().to_vec(),
                    slots,
                    &self.cancel,
                )?)
            }
            (_, Some(b)) => b.clone(),
            _ => empty(),
        };
        let idata = match (
            Self::od2_field(sol, "idata").as_deref(),
            Self::od2_field(&new, "idata").as_deref(),
        ) {
            (Some(Array::Struct(a)), Some(Array::Struct(b))) => {
                let mut slots = Vec::new();
                for name in a.field_names() {
                    let va = Self::od2_field(a, name);
                    let vb = Self::od2_field(b, name);
                    let merged = match (va.as_deref(), vb.as_deref()) {
                        (Some(Array::Numeric(ma)), Some(Array::Numeric(mb))) if !ma.is_empty() => {
                            Arc::new(Array::Numeric(join(ma, mb, n, n2, name == "dif3d")?))
                        }
                        (Some(v), _) => Arc::new(v.clone()),
                        _ => Arc::new(empty()),
                    };
                    slots.push(merged);
                }
                Array::Struct(StructArray::from_slots(
                    1,
                    1,
                    a.field_names().to_vec(),
                    slots,
                    &self.cancel,
                )?)
            }
            _ => empty(),
        };
        let new_ext = Self::od2_field(&new, "extdata")
            .map(|a| (*a).clone())
            .unwrap_or_else(empty);
        let mut names = vec!["solver", "extdata", "stats", "x", "y"];
        let mut values = vec![
            text(&solver)?,
            new_ext,
            stats,
            Array::Numeric(xs),
            Array::Numeric(ys),
        ];
        if Self::od2_field(sol, "xe").is_some() || Self::od2_field(&new, "xe").is_some() {
            for (name, r) in [("xe", 1), ("ye", neq), ("ie", 1)] {
                let old = Self::od2_field(sol, name).and_then(|a| Self::od2_matrix(&a));
                let add = Self::od2_field(&new, name).and_then(|a| Self::od2_matrix(&a));
                let cols =
                    old.as_ref().map_or(0, Matrix::cols) + add.as_ref().map_or(0, Matrix::cols);
                let mut data = Vec::with_capacity(Matrix::checked_len(r, cols)?);
                for m in old.iter().chain(add.iter()) {
                    if (!m.is_empty() && m.rows() != r) || m.data().iter().any(|v| !v.is_finite()) {
                        return Err(invalid());
                    }
                    data.extend_from_slice(m.data());
                }
                names.push(name);
                values.push(if cols == 0 {
                    empty()
                } else {
                    Array::Numeric(Matrix::from_column_vec(r, cols, data)?)
                });
            }
        }
        names.push("idata");
        values.push(idata);
        self.od2_struct(&names, values)
    }

    fn od2_bvpinit(&mut self, args: Vec<Value>, out: &mut Execution) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let args = self.text_arguments(args)?;
        if matches!(args[0], Value::Container(Array::Struct(_))) {
            return Err(
                "bvpinit: extending a previous solution is not supported yet; use bvpxtend".into(),
            );
        }
        let x = self.od2_doubles(&args[0])?;
        if x.iter().any(|v| !v.is_finite()) {
            return Err("The mesh must be finite.".into());
        }
        if x.is_empty() || x[0] == x[x.len() - 1] {
            return Err("The entries of x must satisfy a = x(1) ~= x(end) = b.".into());
        }
        if x[0] < x[x.len() - 1] {
            if x.windows(2).any(|w| w[1] < w[0]) {
                return Err(
                    "The entries of x must satisfy a = x(1) < x(2) < ... < x(end) = b.".into(),
                );
            }
        } else if x.windows(2).any(|w| w[1] > w[0]) {
            return Err("The entries of x must satisfy a = x(1) > x(2) > ... > x(end) = b.".into());
        }
        let extras: Vec<Value> = args.iter().skip(3).cloned().collect();
        let guess = &args[1];
        let ycols: Vec<Vec<f64>> = match self.od2_callable(guess) {
            Some(c) => {
                if let Callable::Name(name) = &c
                    && self.resolve_function(name)?.is_none()
                    && !crate::help::is_supported(name)
                {
                    return Err(format!(
                        "Undefined function '{name}' for input arguments of type 'double'."
                    ));
                }
                let c = match c {
                    Callable::Name(name) => Callable::Handle(FunctionHandle::named(&name)),
                    handle => handle,
                };
                let mut cols = Vec::with_capacity(x.len());
                for xi in &x {
                    self.check_cancel()?;
                    let mut a = vec![tv(*xi)];
                    a.extend(extras.iter().cloned());
                    let v = self.od2_invoke(&c, a, 1, out)?;
                    let m = v
                        .into_iter()
                        .next()
                        .unwrap_or(Value::Void)
                        .matrix(&self.cancel)?;
                    if m.rows() != 1 && m.cols() != 1 {
                        return Err("The guess for the solution must return a vector.".into());
                    }
                    Matrix::checked_len(m.len(), x.len())?;
                    cols.push(m.data().to_vec());
                }
                cols
            }
            _ => {
                let m = guess.clone().matrix(&self.cancel)?;
                if m.rows() != 1 && m.cols() != 1 {
                    return Err("The guess for the solution must be a vector.".into());
                }
                Matrix::checked_len(m.len(), x.len())?;
                vec![m.data().to_vec(); x.len()]
            }
        };
        let neq = ycols[0].len();
        if ycols.iter().any(|c| c.len() != neq) {
            return Err(
                "The guess for the solution must have the same length at every point.".into(),
            );
        }
        let mut names = vec!["solver", "x", "y"];
        let mut values = vec![text("bvpinit")?, row(x.clone())?, columns(&ycols, neq)?];
        if let Some(p) = args.get(2)
            && !Self::od2_is_empty(p)
        {
            names.push("parameters");
            values.push(p.clone().array(&self.cancel)?);
        }
        names.push("yinit");
        values.push(guess.clone().array(&self.cancel)?);
        self.od2_struct(&names, values)
    }

    /// bvpset / ddeset: an option struct with every field, [] where unset.
    fn od2_set(&self, names: &[&'static str], setname: &str, args: Vec<Value>) -> Result<Value> {
        let args = self.text_arguments(args)?;
        let mut slots: Vec<Arc<Array>> = (0..names.len()).map(|_| Arc::new(empty())).collect();
        let mut i = 0;
        while i < args.len() && !matches!(args[i], Value::Text(_)) {
            match &args[i] {
                v if Self::od2_is_empty(v) => {}
                Value::Container(Array::Struct(set)) if set.len() == 1 => {
                    for (slot, name) in slots.iter_mut().zip(names) {
                        if let Some(v) = Self::od2_field(set, name)
                            && !v.is_empty()
                        {
                            *slot = v;
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "Expected argument {} to be a string property name or an options structure created with {setname}.",
                        i + 1
                    ));
                }
            }
            i += 1;
        }
        if !(args.len() - i).is_multiple_of(2) {
            return Err("Arguments must occur in name-value pairs.".into());
        }
        while i < args.len() {
            let Value::Text(name) = &args[i] else {
                return Err(format!(
                    "Expected argument {} to be a string property name.",
                    i + 1
                ));
            };
            let p = property(names, name)?;
            if let Some(at) = names.iter().position(|n| *n == p) {
                slots[at] = Arc::new(args[i + 1].clone().array(&self.cancel)?);
            }
            i += 2;
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names.iter().map(|n| (*n).to_string()).collect(),
            slots,
            &self.cancel,
        )?)))
    }

    fn od2_get(&self, names: &[&'static str], setname: &str, args: Vec<Value>) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        if args.len() > 3 {
            return Err("Too many input arguments.".into());
        }
        let args = self.text_arguments(args)?;
        let default = args.get(2).cloned().unwrap_or(Value::from(empty()));
        if Self::od2_is_empty(&args[0]) {
            return Ok(default);
        }
        let Value::Container(Array::Struct(set)) = &args[0] else {
            return Err(format!(
                "First argument must be an options structure created with {setname}."
            ));
        };
        let Value::Text(name) = &args[1] else {
            return Err("Property name must be a character vector.".into());
        };
        let p = property(names, name).map_err(|e| {
            if e.starts_with("Unrecognized") {
                format!("{e}  See {setname} for possibilities.")
            } else {
                e
            }
        })?;
        Ok(match Self::od2_field(set, p) {
            Some(v) if !v.is_empty() && set.len() == 1 => Value::from((*v).clone()),
            _ => default,
        })
    }

    fn od2_bvp(&mut self, name: &str, args: Vec<Value>, out: &mut Execution) -> Result<Value> {
        if args.len() < 3 {
            return Err("Not enough input arguments.".into());
        }
        let args = self.text_arguments(args)?;
        let (Some(f), Some(bc)) = (self.od2_callable(&args[0]), self.od2_callable(&args[1])) else {
            return Err(format!(
                "{name} expects function handles for ODEFUN and BCFUN"
            ));
        };
        let Value::Container(Array::Struct(init)) = &args[2] else {
            return Err("SOLINIT must be a structure created with BVPINIT.".into());
        };
        let x = Self::od2_field(init, "x")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or("SOLINIT must have fields x and y.")?;
        let y = Self::od2_field(init, "y")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or("SOLINIT must have fields x and y.")?;
        let xv = x.data().to_vec();
        if xv.len() < 2 || xv.windows(2).any(|w| w[0] == w[1]) {
            return Err(format!(
                "{name}: multipoint boundary value problems (repeated mesh points) are not supported yet"
            ));
        }
        let neq = y.rows();
        let ycols: Vec<Vec<f64>> = (0..y.cols())
            .map(|j| y.data()[j * neq..(j + 1) * neq].to_vec())
            .collect();
        Self::od2_validate_mesh(&x, &y)?;
        let p0: Vec<f64> = Self::od2_field(init, "parameters")
            .and_then(|a| Self::od2_matrix(&a))
            .map(|m| m.data().to_vec())
            .unwrap_or_default();
        let opts = self.od2_options(args.get(3), "BVPSET")?;
        Self::od2_unsupported(&opts, &["SingularTerm"], name)?;
        let extras: Vec<Value> = args.iter().skip(4).cloned().collect();
        let rtol = match opts.get("RelTol") {
            None => 1e-3,
            Some(v) => self.od2_scalar(&v, "RelTol")?,
        };
        if rtol <= 0. || !rtol.is_finite() {
            return Err("RelTol must be a positive scalar.".into());
        }
        let rtol = rtol.max(100. * f64::EPSILON);
        let atol = match opts.get("AbsTol") {
            None => vec![1e-6],
            Some(v) => self.od2_numbers(&v)?,
        };
        let nmax = match opts.get("Nmax") {
            None => (10000 / neq.max(1)).max(2),
            Some(v) => self.od2_scalar(&v, "Nmax")?.max(2.) as usize,
        };
        Self::od2_validate_atol(&atol, neq)?;
        let stats_on = matches!(opts.get("Stats"), Some(v) if self.od2_text(&v)? == "on");
        let fjac = match opts.get("FJacobian") {
            Some(a) => match &*a {
                Array::Handle(h) => Some(Callable::Handle(h.clone())),
                _ => None,
            },
            None => None,
        };
        let method = if name == "bvp4c" {
            bvp::Method::Simpson
        } else {
            bvp::Method::Lobatto4
        };
        let result = {
            let mut model = BvpCalls {
                e: self,
                out,
                f,
                bc,
                fjac,
                extras,
                neq,
                npar: p0.len(),
                solver: name.into(),
            };
            bvp::solve(
                &mut model,
                method,
                &xv,
                &ycols,
                &p0,
                &bvp::BvpOpts { rtol, atol, nmax },
            )?
        };
        self.od2_emit_lines(out, result.warnings.clone())?;
        let n = result.x.len();
        let measure = if name == "bvp4c" { "maxres" } else { "maxerr" };
        if stats_on {
            let what = if name == "bvp4c" { "residual" } else { "error" };
            self.od2_emit_lines(
                out,
                vec![
                    format!("The solution was obtained on a mesh of {n} points."),
                    format!("The maximum {what} is {:>10}. ", sci3(result.maxres)),
                    format!(
                        "There were {} calls to the ODE function. ",
                        result.node_evals
                    ),
                    format!("There were {} calls to the BC function. ", result.bc_evals),
                ],
            )?;
        }
        let stats = Array::Struct(StructArray::from_slots(
            1,
            1,
            ["nmeshpoints", measure, "nODEevals", "nBCevals"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            vec![
                num(n as f64),
                num(result.maxres),
                num(result.node_evals as f64),
                num(result.bc_evals as f64),
            ],
            &self.cancel,
        )?);
        let mut names = vec!["solver", "x", "y"];
        let mut values = vec![
            text(name)?,
            row(result.x.clone())?,
            columns(&result.y, neq)?,
        ];
        if name == "bvp4c" {
            names.push("yp");
            values.push(columns(&result.yp, neq)?);
        } else {
            let mut mids = Vec::new();
            for st in &result.stages {
                mids.push(st[0].clone());
                mids.push(st[1].clone());
            }
            // Stage derivatives for the interpolant.
            let ypmid = self.od2_bvp5c_stage_slopes(&result, neq)?;
            let idata = self.od2_struct(
                &["yp", "ypmid"],
                vec![columns(&result.yp, neq)?, columns(&ypmid, neq)?],
            )?;
            names.push("idata");
            values.push(idata.array(&self.cancel)?);
            let _ = mids;
        }
        if !result.p.is_empty() {
            names.push("parameters");
            values.push(column(result.p.clone())?);
        }
        names.push("stats");
        values.push(stats);
        self.od2_struct(&names, values)
    }

    /// Slopes at bvp5c's interior collocation points, recovered from the
    /// stage values through the collocation conditions.
    fn od2_bvp5c_stage_slopes(&self, r: &bvp::BvpOut, neq: usize) -> Result<Vec<Vec<f64>>> {
        // Y2, Y3 satisfy Y_j = y_i + h * sum_l a_jl f_l with known f_1, f_4:
        // solve the 2x2 system for f_2, f_3 per component.
        let s5 = 5f64.sqrt();
        let c = [0., (5. - s5) / 10., (5. + s5) / 10., 1.];
        let a = |j: usize, l: usize| bvp::basis_integral(&c, l, c[j]);
        let mut out = Vec::new();
        for i in 0..r.x.len() - 1 {
            let h = r.x[i + 1] - r.x[i];
            let (a21, a22, a23, a24) = (a(1, 0), a(1, 1), a(1, 2), a(1, 3));
            let (a31, a32, a33, a34) = (a(2, 0), a(2, 1), a(2, 2), a(2, 3));
            let det = a22 * a33 - a23 * a32;
            let mut f2 = vec![0.; neq];
            let mut f3 = vec![0.; neq];
            for l in 0..neq {
                let r2 =
                    (r.stages[i][0][l] - r.y[i][l]) / h - a21 * r.yp[i][l] - a24 * r.yp[i + 1][l];
                let r3 =
                    (r.stages[i][1][l] - r.y[i][l]) / h - a31 * r.yp[i][l] - a34 * r.yp[i + 1][l];
                f2[l] = (r2 * a33 - a23 * r3) / det;
                f3[l] = (a22 * r3 - a32 * r2) / det;
            }
            out.push(f2);
            out.push(f3);
        }
        Ok(out)
    }

    fn od2_bvpxtend(&mut self, args: Vec<Value>) -> Result<Value> {
        if args.len() < 2 {
            return Err("Not enough input arguments.".into());
        }
        let args = self.text_arguments(args)?;
        let Value::Container(Array::Struct(sol)) = &args[0] else {
            return Err("Struct contents reference from a non-struct array object.".into());
        };
        let x = Self::od2_field(sol, "x")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or("Reference to non-existent field 'x'.")?;
        let y = Self::od2_field(sol, "y")
            .and_then(|a| Self::od2_matrix(&a))
            .ok_or("Reference to non-existent field 'y'.")?;
        let xnew = self
            .od2_doubles(&args[1])?
            .first()
            .copied()
            .ok_or("XNEW must be a scalar.")?;
        let n = x.len();
        let neq = y.rows();
        let xs = x.data();
        Self::od2_validate_mesh(&x, &y)?;
        let (lo, hi) = (xs[0].min(xs[n - 1]), xs[0].max(xs[n - 1]));
        if xnew >= lo && xnew <= hi {
            return Err("XNEW inside the previous interval of integration.".into());
        }
        let at_end = (xnew > hi) == (xs[n - 1] > xs[0]);
        let (k, kn) = if at_end { (n - 1, n - 2) } else { (0, 1) };
        let ycol = |j: usize| y.data()[j * neq..(j + 1) * neq].to_vec();
        let yp = Self::od2_field(sol, "yp")
            .or_else(|| {
                let idata = Self::od2_field(sol, "idata")?;
                let Array::Struct(idata) = &*idata else {
                    return None;
                };
                Self::od2_field(idata, "yp")
            })
            .and_then(|a| Self::od2_matrix(&a));
        if yp.as_ref().is_some_and(|m| {
            m.rows() != neq || m.cols() != n || m.data().iter().any(|v| !v.is_finite())
        }) {
            return Err("The solution derivative array must match y.".into());
        }
        let ypcol = |j: usize| {
            yp.as_ref()
                .map(|m| m.data()[j * neq..(j + 1) * neq].to_vec())
        };
        let (ynew, ypnew) = match args.get(2) {
            Some(Value::Text(mode)) => match mode.to_lowercase().as_str() {
                "constant" => (ycol(k), vec![0.; neq]),
                "linear" => {
                    let s = ypcol(k).ok_or("The solution structure has no field yp.")?;
                    (
                        (0..neq)
                            .map(|l| ycol(k)[l] + (xnew - xs[k]) * s[l])
                            .collect(),
                        s,
                    )
                }
                "solution" => {
                    let (s0, s1) = (
                        ypcol(kn).ok_or("The solution structure has no field yp.")?,
                        ypcol(k).ok_or("The solution structure has no field yp.")?,
                    );
                    bvp::hermite(xnew, xs[kn], &ycol(kn), &s0, xs[k], &ycol(k), &s1)
                }
                other => return Err(format!("Unrecognized extrapolation method '{other}'.")),
            },
            Some(v) if !Self::od2_is_empty(v) => {
                let v = self.od2_doubles(v)?;
                if v.len() != neq {
                    return Err(format!("YNEW must be a vector of length {neq}."));
                }
                (v, vec![0.; neq])
            }
            _ => (ycol(k), vec![0.; neq]),
        };
        let extend = |m: &Matrix, add: &[f64]| -> Result<Array> {
            let r = m.rows().max(1);
            let mut data = Vec::with_capacity(Matrix::checked_len(r, m.cols() + 1)?);
            if at_end {
                data.extend_from_slice(m.data());
                data.extend_from_slice(add);
            } else {
                data.extend_from_slice(add);
                data.extend_from_slice(m.data());
            }
            Ok(Array::Numeric(Matrix::from_column_vec(
                r,
                m.cols() + 1,
                data,
            )?))
        };
        let mut result = sol.clone();
        result = result.with_field("x", extend(&x, &[xnew])?, &self.cancel)?;
        result = result.with_field("y", extend(&y, &ynew)?, &self.cancel)?;
        if let Some(m) = &yp {
            if Self::od2_field(sol, "yp").is_some() {
                result = result.with_field("yp", extend(m, &ypnew)?, &self.cancel)?;
            } else if let Some(idata) = Self::od2_field(sol, "idata")
                && let Array::Struct(idata) = &*idata
            {
                let idata = idata.with_field("yp", extend(m, &ypnew)?, &self.cancel)?;
                result = result.with_field("idata", Array::Struct(idata), &self.cancel)?;
            }
        }
        if let Some(p) = args.get(3)
            && !Self::od2_is_empty(p)
        {
            result =
                result.with_field("parameters", p.clone().array(&self.cancel)?, &self.cancel)?;
        }
        Ok(Value::Container(Array::Struct(result)))
    }

    fn od2_dde(&mut self, name: &str, args: Vec<Value>, out: &mut Execution) -> Result<Value> {
        if args.len() < 4 {
            return Err(format!("Not enough input arguments. See {}.", upper(name)));
        }
        let args = self.text_arguments(args)?;
        let Some(f) = self.od2_callable(&args[0]) else {
            return Err(format!("{name} expects a function handle for DDEFUN"));
        };
        let tspan = self.od2_doubles(&args[3])?;
        if tspan.len() < 2 {
            return Err("Must have tspan(1) < tspan(end).".into());
        }
        let delay_fun = if name == "ddesd" {
            self.od2_callable(&args[1])
        } else {
            None
        };
        let lags = if delay_fun.is_some() {
            Vec::new()
        } else {
            let l = self.od2_doubles(&args[1])?;
            if l.is_empty() || l.iter().any(|v| *v <= 0. || !v.is_finite()) {
                return Err("The lags must all be positive.".into());
            }
            l
        };
        let (t0, tfinal) = (tspan[0], tspan[tspan.len() - 1]);
        if tspan.iter().any(|v| !v.is_finite()) || tfinal <= t0 {
            return Err("Must have tspan(1) < tspan(end).".into());
        }
        let opts = self.od2_options(args.get(4), "DDESET")?;
        Self::od2_unsupported(&opts, &["OutputFcn", "OutputSel"], name)?;
        if matches!(opts.get("NormControl"), Some(v) if self.od2_text(&v)? == "on") {
            return Err(format!("{name} option 'NormControl' is not supported yet"));
        }
        let extras: Vec<Value> = args.iter().skip(5).cloned().collect();
        let history_value = args[2].clone();
        let (history, history_fun) = match &history_value {
            Value::Container(Array::Handle(h)) => {
                (dde::History::Function, Some(Callable::Handle(h.clone())))
            }
            Value::Container(Array::Struct(s)) => {
                let x = Self::od2_field(s, "x")
                    .and_then(|a| Self::od2_matrix(&a))
                    .ok_or("The history structure needs fields x, y and yp.")?;
                let y = Self::od2_field(s, "y")
                    .and_then(|a| Self::od2_matrix(&a))
                    .ok_or("The history structure needs fields x, y and yp.")?;
                let yp = Self::od2_field(s, "yp")
                    .and_then(|a| Self::od2_matrix(&a))
                    .ok_or("The history structure needs fields x, y and yp.")?;
                let r = y.rows();
                Self::od2_validate_mesh(&x, &y)?;
                if x.data()[0] > x.data()[x.len() - 1]
                    || yp.rows() != r
                    || yp.cols() != y.cols()
                    || yp.data().iter().any(|v| !v.is_finite())
                {
                    return Err(
                        "The history derivative array must match y on an increasing mesh.".into(),
                    );
                }
                let cols = |m: &Matrix| {
                    (0..m.cols())
                        .map(|j| m.data()[j * r..(j + 1) * r].to_vec())
                        .collect::<Vec<_>>()
                };
                (
                    dde::History::Solution(x.data().to_vec(), cols(&y), cols(&yp)),
                    None,
                )
            }
            v => (dde::History::Constant(self.od2_doubles(v)?), None),
        };
        let initial_y = match opts.get("InitialY") {
            None => None,
            Some(v) => Some(self.od2_numbers(&v)?),
        };
        let neq = match (&initial_y, &history) {
            (Some(v), _) => v.len(),
            (None, dde::History::Constant(v)) => v.len(),
            (None, dde::History::Solution(_, y, _)) => y[0].len(),
            (None, dde::History::Function) => {
                let mut a = vec![tv(t0)];
                a.extend(extras.iter().cloned());
                let hf = history_fun.clone().ok_or("history")?;
                let v = self.od2_invoke(&hf, a, 1, out)?;
                self.od2_vector(v.into_iter().next().unwrap_or(Value::Void))?
                    .len()
            }
        };
        let mut warnings = Vec::new();
        let (rtol, _atol, _nc, threshold, hmax, htry) =
            self.od2_tolerances(&opts, neq, t0, tfinal, &f, &mut warnings)?;
        let jumps = match opts.get("Jumps") {
            None => Vec::new(),
            Some(v) => self.od2_numbers(&v)?,
        };
        if neq == 0 || jumps.iter().any(|v| !v.is_finite()) {
            return Err("DDE initial conditions must be nonempty and jumps must be finite.".into());
        }
        let stats_on = matches!(opts.get("Stats"), Some(v) if self.od2_text(&v)? == "on");
        let events_fun = match opts.get("Events") {
            None => None,
            Some(a) => match &*a {
                Array::Handle(h) => Some(Callable::Handle(h.clone())),
                Array::Character(c) => Some(Callable::Name(c.to_utf8(&self.cancel)?)),
                _ => return Err("The Events option must be a function handle.".into()),
            },
        };
        let o = dde::DdeOpts {
            rtol,
            threshold,
            hmax,
            htry,
            jumps,
            initial_y,
            events: events_fun.is_some(),
        };
        let result = {
            let ndel = match &delay_fun {
                None => lags.len(),
                Some(d) => {
                    let y0 = if let Some(y) = &o.initial_y {
                        y.clone()
                    } else {
                        match &history {
                            dde::History::Constant(v) => v.clone(),
                            dde::History::Solution(_, y, _) => y[y.len() - 1].clone(),
                            dde::History::Function => {
                                let mut a = vec![tv(t0)];
                                a.extend(extras.iter().cloned());
                                let v = self.od2_invoke(
                                    history_fun.as_ref().ok_or("history")?,
                                    a,
                                    1,
                                    out,
                                )?;
                                self.od2_vector(v.into_iter().next().unwrap_or(Value::Void))?
                            }
                        }
                    };
                    let mut a = vec![tv(t0), cv(&y0)];
                    a.extend(extras.iter().cloned());
                    let v = self.od2_invoke(d, a, 1, out)?;
                    self.od2_vector(v.into_iter().next().unwrap_or(Value::Void))?
                        .len()
                }
            };
            let mut model = DdeCalls {
                e: self,
                out,
                f,
                history: history_fun,
                history_solution: match &history_value {
                    Value::Container(Array::Struct(s)) => Some(s.clone()),
                    _ => None,
                },
                delays: delay_fun.clone(),
                events: events_fun.clone(),
                extras,
                neq,
                ndel,
            };
            Matrix::checked_len(neq, ndel)?;
            let delays = match &delay_fun {
                None => dde::Delays::Lags(lags.clone()),
                Some(_) => dde::Delays::Function(ndel),
            };
            let scheme = if name == "dde23" {
                dde::Scheme::Bs23
            } else {
                dde::Scheme::Rk4
            };
            dde::solve(&mut model, scheme, &delays, &history, t0, tfinal, &o)?
        };
        warnings.extend(result.warnings.iter().cloned());
        self.od2_emit_lines(out, warnings)?;
        if stats_on {
            self.od2_emit_lines(out, Self::od2_stats_lines(&result.stats))?;
        }
        let mut names = vec!["solver", "history"];
        let mut values = vec![text(name)?, history_value.array(&self.cancel)?];
        if name == "dde23" {
            names.push("discont");
            values.push(row(result.discont.clone())?);
        }
        names.extend(["x", "y"]);
        values.push(row(result.x.clone())?);
        values.push(columns(&result.y, neq)?);
        if events_fun.is_some() {
            names.extend(["xe", "ye", "ie"]);
            values.extend(if result.te.is_empty() {
                vec![empty(), empty(), empty()]
            } else {
                vec![
                    row(result.te.clone())?,
                    columns(&result.ye, neq)?,
                    row(result.ie.iter().map(|i| *i as f64).collect())?,
                ]
            });
        }
        names.push("stats");
        values.push(self.od2_stats_struct(&result.stats)?);
        names.push("yp");
        values.push(columns(&result.yp, neq)?);
        self.od2_struct(&names, values)
    }
}

/// printf "%.3e" (three decimals, two-digit exponent).
fn sci3(x: f64) -> String {
    if !x.is_finite() {
        return fmt_g(x);
    }
    let s = format!("{x:.3e}");
    let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = e.parse().unwrap_or(0);
    format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

#[cfg(test)]
mod cleanroom_tests {
    use super::{Callable, Engine, OptionSet, StructArray, interp, num};

    #[test]
    fn step_options_use_magnitudes_and_cap_maxstep_at_the_span() {
        let engine = Engine::default();
        let function = Callable::Name("decay".into());
        let options = |fields: Vec<(&str, f64)>| OptionSet {
            set: Some(
                StructArray::from_slots(
                    1,
                    1,
                    fields.iter().map(|(name, _)| (*name).into()).collect(),
                    fields.iter().map(|(_, value)| num(*value)).collect(),
                    &engine.cancel,
                )
                .unwrap(),
            ),
        };
        for (t0, tf) in [(0., 1.), (1., 0.)] {
            for (value, expected) in [
                (f64::INFINITY, 1.),
                (f64::NEG_INFINITY, 1.),
                (f64::NAN, 1.),
                (-0.1, 0.1),
                (-2., 1.),
            ] {
                let options = options(vec![("MaxStep", value), ("InitialStep", -0.01)]);
                let (_, _, _, _, maxstep, initial) = engine
                    .od2_tolerances(&options, 1, t0, tf, &function, &mut vec![])
                    .unwrap();
                assert_eq!(maxstep, expected);
                assert_eq!(initial, Some(0.01));
            }
        }
        for option in ["MaxStep", "InitialStep"] {
            let options = options(vec![(option, 0.)]);
            let error = engine
                .od2_tolerances(&options, 1, 0., 1., &function, &mut vec![])
                .unwrap_err();
            assert!(error.contains("must be greater than zero"), "{error}");
        }
    }

    #[test]
    fn continuous_extensions_reproduce_polynomials_and_reverse_time_slopes() {
        let (value, derivative) = interp::rk(&[[0., 1., 0., 0.]], &[&[2.]], 1., &[3.], -0.5, 0.75);
        assert_eq!(value, [2.75]);
        assert_eq!(derivative, [2.]);
        // Hermite representation of y=t^3 with a negative step.
        for x in [0., 0.25, 0.75, 1.] {
            let (value, derivative) = interp::tr(x, 1., &[1.], -1., &[0.], &[-3.], &[0.]);
            assert!((value[0] - x.powi(3)).abs() < 1e-14);
            assert!((derivative[0] - 3. * x * x).abs() < 2e-9);
        }
        // A quadratic through unequal nodes, including evaluation at nodes.
        for x in [1., 0.2, -1., 0.6] {
            let (value, derivative) = interp::poly(x, &[1., 0.2, -1.], &[&[1.], &[0.04], &[1.]]);
            assert!((value[0] - x * x).abs() < 1e-14);
            assert!((derivative[0] - 2. * x).abs() < 2e-9);
        }
    }

    #[test]
    fn backward_difference_extension_has_the_expected_linear_slope() {
        for h in [-0.125, 0.125] {
            for x in [0.9, 1., 1.1] {
                let (value, derivative) =
                    interp::ndf(x, 1., &[2.], h, &[vec![0.], vec![3. * h]], 1);
                assert!((value[0] - (2. + 3. * (x - 1.))).abs() < 1e-14);
                assert!((derivative[0] - 3.).abs() < 1e-9);
            }
        }
    }
}

#[cfg(test)]
mod cleanroom_short_step_tests {
    use super::interp;

    #[test]
    fn slope_keeps_short_step_extensions_local() {
        for h in [-1e-12, 1e-12] {
            let derivative = interp::slope(&|x| vec![(x / h).powi(5)], 0., h);
            assert!(derivative[0].abs() < 2e-8, "{derivative:?}");
        }
    }
}
