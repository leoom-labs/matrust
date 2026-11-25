//! Linear time-invariant VALUE objects, deliberately separate from ObjTable's
//! handle objects. A tagged StructArray owns immutable, shared column-major
//! property arrays. Assignment clones slot pointers; a property write validates
//! and replaces the facade, so aliases (including nested cells) remain unchanged.
//!
//! Mathematics: Franklin, Powell & Emami-Naeini, Feedback Control of Dynamic
//! Systems, 8th ed., chs. 3, 7, 8; Ogata, Modern Control Engineering, 5th ed.,
//! chs. 2, 9 (rational models, state realizations and feedback algebra).
//! Sampling: Van Loan, "Computing Integrals Involving the Matrix Exponential",
//! IEEE TAC 23 (1978), pp. 395-404, doi:10.1109/TAC.1978.1101743.
//! The augmented exponential integrates constant/linear inputs without A^-1.
//! minreal uses explicitly tolerance-limited pole/zero cancellation, not a
//! controllability/observability staircase or a guarantee of numerical rank.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, Complex64 as C, ComplexMatrix, LogicalMatrix, Matrix, Result,
    StructArray,
    syntax::{PathStep, Token},
};
use std::sync::{Arc, atomic::AtomicBool};

mod delays;
mod interconnect;
pub(super) use interconnect::literal;
mod mimo;
pub(super) use mimo::{evalfr, frequency_coefficients, index, io_shape};
mod properties;
use properties::{
    apply_options, copy_metadata, delay_check, joined_states, names_check, property_name,
    props_decode, public_properties, replace_properties,
};
pub(super) use properties::{lti_set_options, private_property, property_facade};

const TAG: &str = "MatRustLTI";
const MAX_DEGREE: usize = 256;
const MAX_STATES: usize = 1024;
const MAX_ENTRIES: usize = 1_048_576;
pub(super) const NAMES: &[&str] = &[
    "tf", "zpk", "ss", "tfdata", "zpkdata", "ssdata", "feedback", "series", "parallel", "append",
    "pole", "zero", "dcgain", "minreal", "c2d", "d2c", "isstable", "order", "damp", "pade", "pid",
    "pidstd", "issiso",
];
pub(super) fn data_name(n: &str) -> bool {
    matches!(n, "tfdata" | "zpkdata" | "ssdata" | "damp" | "pade")
}
pub(super) fn class(a: &Array) -> Option<&'static str> {
    let Array::Struct(s) = a else { return None };
    if s.len() != 1 {
        return None;
    }
    let Array::Character(c) = s.get(s.field_index(TAG)?, 0).ok()?.as_ref() else {
        return None;
    };
    match c.units() {
        [116, 102] => Some("tf"),
        [122, 112, 107] => Some("zpk"),
        [115, 115] => Some("ss"),
        [112, 105, 100] => Some("pid"),
        [112, 105, 100, 115, 116, 100] => Some("pidstd"),
        _ => None,
    }
}
pub(super) fn is_lti(v: &Value) -> bool {
    matches!(v, Value::Container(a) if class(a).is_some())
}
#[derive(Clone)]
pub(super) enum Model {
    Tf(Matrix, Matrix),
    Zpk(Vec<C>, Vec<C>, f64),
    Ss(Matrix, Matrix, Matrix, Matrix),
    TfArray(CellArray, CellArray),
    ZpkArray(CellArray, CellArray, Matrix),
}
#[derive(Clone)]
pub(super) struct Lti {
    pub(super) model: Model,
    pub(super) ts: f64,
    variable: String,
    input: Array,
    output: Array,
    props: LtiProperties,
}
#[derive(Clone, Default)]
struct LtiProperties {
    delay: [f64; 3],
    name: Option<Array>,
    state: Option<Array>,
    notes: Option<Array>,
    user_data: Option<Array>,
    paths: Vec<Lti>,
}

const DELAYS: [&str; 3] = ["InputDelay", "OutputDelay", "IODelay"];

impl Lti {
    fn kind(&self) -> &'static str {
        match self.model {
            Model::Tf(..) | Model::TfArray(..) => "tf",
            Model::Zpk(..) | Model::ZpkArray(..) => "zpk",
            Model::Ss(..) => "ss",
        }
    }
    pub(super) fn ports(&self) -> (usize, usize) {
        match &self.model {
            Model::Ss(_, b, c, _) => (b.cols(), c.rows()),
            Model::TfArray(c, _) | Model::ZpkArray(c, ..) => (c.cols(), c.rows()),
            _ => (1, 1),
        }
    }
    pub(super) fn response_port_labels(&self) -> (Vec<String>, Vec<String>) {
        let (nu, ny) = self.ports();
        (
            port_labels(&self.input, nu, |i| format!("In({i})")),
            port_labels(&self.output, ny, |i| format!("Out({i})")),
        )
    }
    pub(super) fn has_internal_delays(&self) -> bool {
        !self.props.paths.is_empty()
    }
    pub(super) fn require_delay_free(&self, operation: &str) -> Result<()> {
        if self.delay() != 0. || self.has_internal_delays() {
            return Err(format!(
                "{operation} requires a delay-free model; use pade(sys,N) for continuous-time transport delays or absorb discrete sample delays into the model"
            ));
        }
        Ok(())
    }
    pub(super) fn internal_paths(&self) -> &[Lti] {
        &self.props.paths
    }
    pub(super) fn full_delay_paths(&self) -> Vec<Lti> {
        delays::full_paths(self)
    }
    pub(super) fn delays(&self) -> [f64; 3] {
        self.props.delay
    }
    pub(super) fn delay(&self) -> f64 {
        self.props.delay.iter().sum()
    }
    pub(super) fn sample_period(&self) -> f64 {
        if self.ts == -1. { 1. } else { self.ts }
    }
    pub(super) fn states(&self) -> usize {
        match &self.model {
            Model::Ss(a, ..) => a.rows(),
            _ => 0,
        }
    }
    pub(super) fn rational_grid(&self) -> bool {
        matches!(self.model, Model::TfArray(..) | Model::ZpkArray(..))
    }
    pub(super) fn require_siso(&self, operation: &str) -> Result<()> {
        if self.ports() == (1, 1) {
            Ok(())
        } else {
            Err(format!("{operation} requires a SISO LTI model"))
        }
    }
}
fn field(s: &StructArray, name: &str) -> Result<Array> {
    Ok(s.get(
        s.field_index(name)
            .ok_or_else(|| format!("Invalid LTI property {name}"))?,
        0,
    )?
    .as_ref()
    .clone())
}
fn scalar(v: &Value) -> Result<f64> {
    match v {
        Value::Matrix(m) => m.as_scalar(),
        _ => Err("Expected a real double scalar".into()),
    }
}
fn text(v: &Value) -> Result<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
        Value::Container(Array::Str(s)) if s.len() == 1 => s.data()[0]
            .as_deref()
            .map(str::to_owned)
            .ok_or_else(|| "Expected nonmissing scalar text".into()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Ok(String::from_utf16_lossy(c.units()))
        }
        _ => Err("Expected character-row text".into()),
    }
}
fn real(v: &Value) -> Result<Matrix> {
    let Value::Matrix(m) = v else {
        return Err("LTI matrices and coefficients must be real double arrays".into());
    };
    if m.is_nd() || m.len() > MAX_ENTRIES || m.data().iter().any(|x| !x.is_finite()) {
        return Err("LTI matrix exceeds its size bound or contains nonfinite data".into());
    }
    Ok(m.clone())
}
fn unwrap_cell(a: &Array) -> Result<Array> {
    if let Array::Cell(c) = a {
        if c.len() != 1 {
            return Err("Expected a numeric vector or a single coefficient or root cell".into());
        }
        Ok(c.get_linear(0)?.as_ref().clone())
    } else {
        Ok(a.clone())
    }
}
fn coefficient_row(v: &Value) -> Result<Matrix> {
    let v = match v {
        Value::Container(a) => Value::from(unwrap_cell(a)?),
        _ => v.clone(),
    };
    let m = real(&v)?;
    if m.is_empty() || (m.rows() != 1 && m.cols() != 1) || m.len() > MAX_DEGREE + 1 {
        return Err("Polynomial must be a nonempty vector of degree at most 256".into());
    }
    m.reshape(1, m.len())
}
fn trim_leading(m: Matrix) -> Result<Matrix> {
    let i = m
        .data()
        .iter()
        .position(|x| *x != 0.)
        .unwrap_or(m.len() - 1);
    if i == 0 && m.rows() == 1 {
        return Ok(m);
    }
    Matrix::from_column_slice(1, m.len() - i, &m.data()[i..])
}
fn coefficients(v: &Value) -> Result<Matrix> {
    trim_leading(coefficient_row(v)?)
}
fn tf_storage(n: &Value, d: &Value, variable: &str) -> Result<Model> {
    let (n, d) = (coefficient_row(n)?, coefficient_row(d)?);
    let (n, d) = if inverse_variable(variable) {
        let len = n.len().max(d.len());
        (
            pad_coefficients(&n, len, false)?,
            pad_coefficients(&d, len, false)?,
        )
    } else {
        (n, d)
    };
    let (n, d) = (trim_leading(n)?, trim_leading(d)?);
    if d.data()[0] == 0. {
        return Err("Transfer-function denominator cannot be zero".into());
    }
    Ok(Model::Tf(n, d))
}
fn pad_coefficients(p: &Matrix, len: usize, left: bool) -> Result<Matrix> {
    if len <= p.len() {
        return Ok(p.clone());
    }
    let mut data = vec![0.; len];
    let start = if left { len - p.len() } else { 0 };
    data[start..start + p.len()].copy_from_slice(p.data());
    row(data)
}
fn public_tf(n: &Matrix, d: &Matrix, variable: &str) -> Result<(Matrix, Matrix)> {
    let len = n.len().max(d.len());
    Ok((
        pad_coefficients(n, len, true)?,
        if inverse_variable(variable) {
            pad_coefficients(d, len, true)?
        } else {
            d.clone()
        },
    ))
}
fn inverse_variable(name: &str) -> bool {
    matches!(name, "z^-1" | "q^-1")
}
fn default_variable(ts: f64) -> &'static str {
    if ts == 0. { "s" } else { "z" }
}
fn variable_check(variable: &str, ts: f64) -> Result<()> {
    if ts == 0. && matches!(variable, "s" | "p")
        || ts != 0. && matches!(variable, "z" | "q" | "z^-1" | "q^-1")
    {
        Ok(())
    } else {
        Err("Variable must be 's' or 'p' for continuous-time models, or 'z', 'q', 'z^-1', or 'q^-1' for discrete-time models".into())
    }
}
fn roots_input(v: &Value) -> Result<Vec<C>> {
    let v = match v {
        Value::Container(a) => Value::from(unwrap_cell(a)?),
        _ => v.clone(),
    };
    let (r, c, len) = match &v {
        Value::Complex(m) if !m.is_nd() => (m.rows(), m.cols(), m.len()),
        Value::Matrix(m) if !m.is_nd() => (m.rows(), m.cols(), m.len()),
        _ => return Err("LTI roots must be real or complex double vectors".into()),
    };
    // Reject inadmissible inputs before copying/converting any root payload.
    if len > MAX_DEGREE || (len != 0 && r != 1 && c != 1) {
        return Err("LTI roots must be finite vectors of at most 256 entries".into());
    }
    let z = match &v {
        Value::Complex(m) => m.data().to_vec(),
        _ => {
            let m = real(&v)?;
            m.data().iter().map(|&x| C::new(x, 0.)).collect()
        }
    };
    if z.iter().any(|x| !x.re.is_finite() || !x.im.is_finite()) {
        return Err("LTI roots must be finite vectors of at most 256 entries".into());
    }
    Ok(z)
}
fn ts_check(ts: f64) -> Result<()> {
    if ts.is_finite() && (ts >= 0. || ts == -1.) {
        Ok(())
    } else {
        Err(
            "Ts must be zero (continuous), a positive finite sample time, or -1 (unspecified)"
                .into(),
        )
    }
}
fn normalize(n: Matrix, d: Matrix) -> Result<Model> {
    if n.data().iter().chain(d.data()).any(|x| !x.is_finite()) {
        return Err("LTI coefficients must be finite".into());
    }
    if d.data()[0] == 0. {
        return Err("Transfer-function denominator cannot be zero".into());
    }
    let lead = d.data()[0];
    if lead == 1. {
        return Ok(Model::Tf(n, d));
    }
    let n = n.map(|x| x / lead);
    let d = d.map(|x| x / lead);
    if n.data().iter().chain(d.data()).any(|x| !x.is_finite()) {
        return Err("Polynomial normalization overflowed".into());
    }
    Ok(Model::Tf(n, d))
}
fn ss_entries(n: usize, ny: usize, nu: usize) -> usize {
    n * n + n * nu + ny * n + ny * nu
}
fn ss_size_check(n: usize, ny: usize, nu: usize) -> Result<()> {
    if n > MAX_STATES || nu > 4096 || ny > 4096 {
        return Err("ss exceeds 1024 states or 4096 ports per axis".into());
    }
    if ss_entries(n, ny, nu) > MAX_ENTRIES {
        return Err("ss exceeds 1048576 total matrix entries".into());
    }
    Ok(())
}
fn ss_check(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix) -> Result<()> {
    let n = a.rows();
    if a.cols() != n
        || b.rows() != n
        || c.cols() != n
        || d.rows() != c.rows()
        || d.cols() != b.cols()
    {
        return Err(
            "ss requires square A, B with n rows, C with n columns, and D sized outputs by inputs"
                .into(),
        );
    }
    ss_size_check(n, c.rows(), b.cols())?;
    if a.data()
        .iter()
        .chain(b.data())
        .chain(c.data())
        .chain(d.data())
        .any(|x| !x.is_finite())
    {
        return Err(
            "State-space matrices must be finite; the transformation may be singular or overflowed"
                .into(),
        );
    }
    Ok(())
}
fn row(v: Vec<f64>) -> Result<Matrix> {
    Matrix::from_column_vec(1, v.len(), v)
}
fn conv(a: &[f64], b: &[f64]) -> Result<Vec<f64>> {
    if a.len() + b.len() > MAX_DEGREE + 2 {
        return Err("LTI polynomial degree exceeds 256".into());
    }
    let mut c = vec![0.; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            c[i + j] += x * y;
        }
    }
    Ok(c)
}
fn addpoly(a: &[f64], b: &[f64], sign: f64) -> Vec<f64> {
    let n = a.len().max(b.len());
    let mut c = vec![0.; n];
    for (i, &x) in a.iter().enumerate() {
        c[n - a.len() + i] += x;
    }
    for (i, &x) in b.iter().enumerate() {
        c[n - b.len() + i] += sign * x;
    }
    c
}
fn poly_roots(z: &[C]) -> Result<Matrix> {
    let mut c = vec![C::new(1., 0.)];
    for &root in z {
        let mut next = vec![C::new(0., 0.); c.len() + 1];
        for (i, &x) in c.iter().enumerate() {
            next[i] += x;
            next[i + 1] -= x * root;
        }
        c = next;
    }
    let scale = c.iter().map(|x| x.norm()).fold(1., f64::max);
    if c.iter()
        .any(|x| x.im.abs() > 1e-10 * scale || !x.re.is_finite() || !x.im.is_finite())
    {
        return Err(
            "Real LTI models require conjugate-paired complex roots and finite coefficients".into(),
        );
    }
    row(c.iter().map(|x| x.re).collect())
}
fn evalpoly(c: &[f64], x: f64) -> f64 {
    c.iter().fold(0., |y, c| y * x + c)
}
fn cancellation_groups(e: &Engine, mut roots: Vec<C>, tol: f64) -> Result<Vec<(C, Option<C>)>> {
    // A repeated real root can split into a tiny conjugate pair numerically.
    // Use the requested cancellation tolerance to project near-real roots,
    // then keep every remaining conjugate pair together during matching.
    for root in &mut roots {
        if root.im.abs() <= tol * (1. + root.norm()) {
            root.im = 0.;
        }
    }
    let mut groups = Vec::with_capacity(roots.len());
    while !roots.is_empty() {
        e.check_cancel()?;
        let root = roots.remove(0);
        let partner = if root.im == 0. {
            None
        } else {
            roots
                .iter()
                .enumerate()
                .filter(|(_, p)| (root.im > 0. && p.im < 0.) || (root.im < 0. && p.im > 0.))
                .min_by(|(_, a), (_, b)| {
                    (**a - root.conj())
                        .norm()
                        .total_cmp(&(**b - root.conj()).norm())
                })
                .map(|(i, _)| i)
        };
        let partner = partner.map(|i| roots.remove(i));
        groups.push(match partner {
            Some(p) if root.im < 0. => (p, Some(root)),
            _ => (root, partner),
        });
    }
    Ok(groups)
}
fn rank(kind: &str) -> u8 {
    match kind {
        "ss" => 2,
        "zpk" => 1,
        _ => 0,
    }
}

pub(super) fn decode(e: &Engine, a: &Array) -> Result<Option<Lti>> {
    let Some(kind) = class(a) else {
        return Ok(None);
    };
    if let Some((p, input, output)) = pid_decode(a)? {
        let sys = Lti {
            model: pid_model(&p)?,
            ts: p.ts,
            variable: if p.ts == 0. { "s" } else { "z" }.into(),
            input,
            output,
            props: LtiProperties::default(),
        };
        names_check(&sys.input, 1)?;
        names_check(&sys.output, 1)?;
        return Ok(Some(sys));
    }
    let Array::Struct(s) = a else { unreachable!() };
    let get = |n| field(s, n);
    let ts = scalar(&get("Ts")?.into())?;
    ts_check(ts)?;
    let variable = text(&get("Variable")?.into())?;
    variable_check(&variable, ts)?;
    let model = match kind {
        "tf" => mimo::tf_model(
            e,
            &get("Numerator")?.into(),
            &get("Denominator")?.into(),
            &variable,
            true,
        )?,
        "zpk" => mimo::zpk_model(
            e,
            &get("Z")?.into(),
            &get("P")?.into(),
            &get("K")?.into(),
            true,
        )?,
        _ => {
            let a = real(&get("A")?.into())?;
            let b = real(&get("B")?.into())?;
            let c = real(&get("C")?.into())?;
            let d = real(&get("D")?.into())?;
            ss_model(a, b, c, d)?
        }
    };
    let mut sys = Lti {
        model,
        ts,
        variable,
        input: get("InputName")?,
        output: get("OutputName")?,
        props: props_decode(s)?,
    };
    let (ni, no) = sys.ports();
    names_check(&sys.input, ni)?;
    names_check(&sys.output, no)?;
    if let Some(state) = &sys.props.state {
        names_check(state, sys.states()).map_err(|_| "StateName must have one name per state")?;
    }
    delay_check(&sys)?;
    sys.props.paths = delays::decode_paths(e, s, &sys)?;
    e.check_cancel()?;
    Ok(Some(sys))
}
fn cell(e: &Engine, a: Array) -> Result<Array> {
    Ok(CellArray::from_shared(1, 1, vec![Arc::new(a)], &e.cancel)?.into())
}
fn char_array(e: &Engine, s: &str) -> Result<Array> {
    Ok(CharacterRow::from_text(s, &e.cancel)?.into())
}
fn empty_names(e: &Engine, n: usize) -> Result<Array> {
    let empty = Arc::new(char_array(e, "")?);
    Ok(CellArray::from_shared(n, 1, vec![empty; n], &e.cancel)?.into())
}
fn new(e: &Engine, model: Model, ts: f64) -> Result<Lti> {
    ts_check(ts)?;
    let mut sys = Lti {
        model,
        ts,
        variable: if ts == 0. { "s" } else { "z" }.into(),
        input: char_array(e, "")?,
        output: char_array(e, "")?,
        props: LtiProperties::default(),
    };
    let (ni, no) = sys.ports();
    sys.input = empty_names(e, ni)?;
    sys.output = empty_names(e, no)?;
    Ok(sys)
}
fn roots_value(cancel: &AtomicBool, z: &[C]) -> Result<Value> {
    if z.iter().all(|x| x.im == 0.) {
        Ok(Value::Matrix(Matrix::from_column_vec(
            z.len(),
            1,
            z.iter().map(|x| x.re).collect(),
        )?))
    } else {
        Ok(Value::Complex(ComplexMatrix::from_column_slice(
            z.len(),
            1,
            z,
            cancel,
        )?))
    }
}
pub(super) fn encode(e: &Engine, s: &Lti) -> Result<Value> {
    let mut fields = vec![
        (TAG, char_array(e, s.kind())?),
        ("Ts", Matrix::scalar(s.ts).into()),
        ("Variable", char_array(e, &s.variable)?),
        ("InputName", s.input.clone()),
        ("OutputName", s.output.clone()),
    ];
    for (name, delay) in DELAYS.into_iter().zip(s.props.delay) {
        fields.push((name, Matrix::scalar(delay).into()));
    }
    fields.extend([
        ("Name", s.props.name.clone().unwrap_or(char_array(e, "")?)),
        ("Notes", s.props.notes.clone().unwrap_or(empty_names(e, 0)?)),
        (
            "UserData",
            s.props
                .user_data
                .clone()
                .unwrap_or(Matrix::filled(0, 0, 0.)?.into()),
        ),
    ]);
    match &s.model {
        Model::Tf(n, d) => {
            let (n, d) = public_tf(n, d, &s.variable)?;
            fields.push(("Numerator", cell(e, n.into())?));
            fields.push(("Denominator", cell(e, d.into())?));
        }
        Model::Zpk(z, p, k) => {
            fields.push(("Z", cell(e, roots_value(&e.cancel, z)?.array(&e.cancel)?)?));
            fields.push(("P", cell(e, roots_value(&e.cancel, p)?.array(&e.cancel)?)?));
            fields.push(("K", Matrix::scalar(*k).into()));
        }
        Model::TfArray(n, d) => {
            fields.push(("Numerator", n.clone().into()));
            fields.push(("Denominator", d.clone().into()));
        }
        Model::ZpkArray(z, p, k) => {
            fields.push(("Z", z.clone().into()));
            fields.push(("P", p.clone().into()));
            fields.push(("K", k.clone().into()));
        }
        Model::Ss(a, b, c, d) => {
            fields.push((
                "StateName",
                match &s.props.state {
                    Some(names) if names_check(names, a.rows()).is_ok() => names.clone(),
                    _ => empty_names(e, a.rows())?,
                },
            ));
            fields.extend([
                ("A", a.clone().into()),
                ("B", b.clone().into()),
                ("C", c.clone().into()),
                ("D", d.clone().into()),
            ]);
        }
    }
    if s.has_internal_delays() {
        fields.push((delays::PATHS, delays::encode_paths(e, s.internal_paths())?));
    }
    let (names, slots) = fields
        .into_iter()
        .map(|(n, a)| (n.into(), Arc::new(a)))
        .unzip();
    Ok(Value::Container(
        StructArray::from_slots(1, 1, names, slots, &e.cancel)?.into(),
    ))
}
fn get_lti(e: &Engine, v: &Value) -> Result<Lti> {
    if let Value::Container(a) = v
        && let Some(s) = decode(e, a)?
    {
        return Ok(s);
    }
    let k = scalar(v)?;
    if !k.is_finite() {
        return Err("An LTI scalar gain must be finite".into());
    }
    new(e, normalize(Matrix::scalar(k), Matrix::scalar(1.))?, 0.)
}

// PID controller objects. They are stored like the other LTI values (a tagged
// struct of immutable property slots) but keep their gains as properties;
// every LTI operation sees them through `pid_model`, the single conversion
// to a transfer function (parallel form C = Kp + Ki/s + Kd s/(Tf s + 1);
// standard form Kp (1 + 1/(Ti s) + Td s/((Td/N) s + 1))). Discrete forms
// replace 1/s by the forward-Euler integrator Ts/(z-1).
const PID_PARALLEL: [&str; 4] = ["Kp", "Ki", "Kd", "Tf"];
const PID_STANDARD: [&str; 4] = ["Kp", "Ti", "Td", "N"];
const FORWARD_EULER: &str = "ForwardEuler";
#[derive(Clone, Copy)]
struct Pid {
    standard: bool,
    gains: [f64; 4],
    ts: f64,
}
fn pid_kind(a: &Array) -> Option<bool> {
    match class(a)? {
        "pid" => Some(false),
        "pidstd" => Some(true),
        _ => None,
    }
}
fn pid_check(p: &Pid) -> Result<()> {
    if p.ts < 0. {
        return Err("PID Ts must be zero or a positive finite sampling interval".into());
    }
    ts_check(p.ts)?;
    let [a, b, c, d] = p.gains;
    let name = if p.standard { "pidstd" } else { "pid" };
    if p.standard {
        if !a.is_finite() || !c.is_finite() || c < 0. {
            return Err("pidstd: Kp must be finite and Td finite and nonnegative".into());
        }
        if b.is_nan() || b <= 0. || d.is_nan() || d <= 0. {
            return Err("pidstd: Ti and N must be positive (Inf removes the term)".into());
        }
    } else if p.gains.iter().any(|x| !x.is_finite()) || d < 0. {
        return Err(format!(
            "{name}: Kp, Ki, Kd and Tf must be finite real scalars with Tf >= 0"
        ));
    }
    Ok(())
}
/// Parallel gains [Kp, Ki, Kd, Tf] of either form.
fn pid_parallel(p: &Pid) -> [f64; 4] {
    if !p.standard {
        return p.gains;
    }
    let [kp, ti, td, n] = p.gains;
    let ki = if ti.is_infinite() { 0. } else { kp / ti };
    let tf = if n.is_infinite() || td == 0. {
        0.
    } else {
        td / n
    };
    [kp, ki, kp * td, tf]
}
fn pid_formula(text: &str, discrete: bool, what: &str) -> Result<()> {
    if (text.is_empty() && !discrete) || text == FORWARD_EULER {
        Ok(())
    } else {
        Err(format!(
            "{what} '{text}' is not supported yet; discrete PID controllers use 'ForwardEuler'"
        ))
    }
}
fn pid_decode(a: &Array) -> Result<Option<(Pid, Array, Array)>> {
    let Some(standard) = pid_kind(a) else {
        return Ok(None);
    };
    let Array::Struct(s) = a else { unreachable!() };
    let names = if standard { PID_STANDARD } else { PID_PARALLEL };
    let mut gains = [0.; 4];
    for (g, n) in gains.iter_mut().zip(names) {
        *g = scalar(&field(s, n)?.into())?;
    }
    let p = Pid {
        standard,
        gains,
        ts: scalar(&field(s, "Ts")?.into())?,
    };
    pid_check(&p)?;
    for what in ["IFormula", "DFormula"] {
        pid_formula(&text(&field(s, what)?.into())?, p.ts > 0., what)?;
    }
    Ok(Some((p, field(s, "InputName")?, field(s, "OutputName")?)))
}
fn pid_model(p: &Pid) -> Result<Model> {
    let [kp, ki, kd, tf] = pid_parallel(p);
    let ts = p.ts;
    // Sum of rationals: Kp + Ki * I(x) + Kd * D(x).
    let mut num = vec![kp];
    let mut den = vec![1.];
    let mut add = |n: Vec<f64>, d: Vec<f64>| -> Result<()> {
        let left = conv(&num, &d)?;
        let right = conv(&n, &den)?;
        num = addpoly(&left, &right, 1.);
        den = conv(&den, &d)?;
        Ok(())
    };
    if ki != 0. {
        if ts == 0. {
            add(vec![ki], vec![1., 0.])?;
        } else {
            add(vec![ki * ts], vec![1., -1.])?;
        }
    }
    if kd != 0. {
        match (ts == 0., tf == 0.) {
            (true, true) => add(vec![kd, 0.], vec![1.])?,
            (true, false) => add(vec![kd, 0.], vec![tf, 1.])?,
            // Kd / (Tf + Ts/(z-1)) = Kd (z-1) / (Tf z + Ts - Tf)
            (false, false) => add(vec![kd, -kd], vec![tf, ts - tf])?,
            (false, true) => add(vec![kd / ts, -kd / ts], vec![1.])?,
        }
    }
    normalize(
        coefficients(&Value::Matrix(row(num)?))?,
        coefficients(&Value::Matrix(row(den)?))?,
    )
}
fn pid_encode(e: &Engine, p: &Pid, input: Array, output: Array) -> Result<Value> {
    let names = if p.standard {
        PID_STANDARD
    } else {
        PID_PARALLEL
    };
    let formula = if p.ts > 0. { FORWARD_EULER } else { "" };
    let mut fields = vec![(
        TAG,
        char_array(e, if p.standard { "pidstd" } else { "pid" })?,
    )];
    for (n, g) in names.iter().zip(p.gains) {
        fields.push((n, Matrix::scalar(g).into()));
    }
    fields.extend([
        ("Ts", Matrix::scalar(p.ts).into()),
        ("IFormula", char_array(e, formula)?),
        ("DFormula", char_array(e, formula)?),
        ("InputName", input),
        ("OutputName", output),
    ]);
    let (names, slots) = fields
        .into_iter()
        .map(|(n, a)| (n.into(), Arc::new(a)))
        .unzip();
    Ok(Value::Container(
        StructArray::from_slots(1, 1, names, slots, &e.cancel)?.into(),
    ))
}
/// A new continuous or discrete parallel-form PID object (used by pidtune).
pub(super) fn pid_value(e: &Engine, gains: [f64; 4], ts: f64) -> Result<Value> {
    let p = Pid {
        standard: false,
        gains,
        ts,
    };
    pid_check(&p)?;
    pid_encode(e, &p, char_array(e, "")?, char_array(e, "")?)
}
fn pid_constructor(e: &Engine, name: &str, args: &[Value]) -> Result<Value> {
    let standard = name == "pidstd";
    if let [v] = args
        && let Value::Container(a) = v
        && let Some((p, input, output)) = pid_decode(a)?
    {
        // pid(Cstd) / pidstd(Cpar): convert between the two forms.
        let gains = if standard == p.standard {
            p.gains
        } else if standard {
            let [kp, ki, kd, tf] = p.gains;
            if kp == 0. && (ki != 0. || kd != 0.) {
                return Err("pidstd: a controller with Kp = 0 has no standard form".into());
            }
            let ti = if ki == 0. { f64::INFINITY } else { kp / ki };
            let td = if kp == 0. { 0. } else { kd / kp };
            let n = if tf == 0. || td == 0. {
                f64::INFINITY
            } else {
                td / tf
            };
            [kp, ti, td, n]
        } else {
            pid_parallel(&p)
        };
        let q = Pid {
            standard,
            gains,
            ts: p.ts,
        };
        pid_check(&q)?;
        return pid_encode(e, &q, input, output);
    }
    let numeric = args
        .iter()
        .take_while(|v| matches!(v, Value::Matrix(_)))
        .count();
    if numeric == 0 || numeric > 5 {
        return Err(format!(
            "{name}: expected {}",
            if standard {
                "pidstd(Kp[,Ti[,Td[,N[,Ts]]]])"
            } else {
                "pid(Kp[,Ki[,Kd[,Tf[,Ts]]]])"
            }
        ));
    }
    let mut gains = if standard {
        [1., f64::INFINITY, 0., f64::INFINITY]
    } else {
        [1., 0., 0., 0.]
    };
    for (g, v) in gains.iter_mut().zip(&args[..numeric.min(4)]) {
        *g = scalar(v).map_err(|_| format!("{name}: gains must be real double scalars"))?;
    }
    let ts = if numeric == 5 { scalar(&args[4])? } else { 0. };
    let rest = &args[numeric..];
    if !rest.len().is_multiple_of(2) {
        return Err(format!("{name}: options must be name-value pairs"));
    }
    for pair in rest.chunks(2) {
        let option = text(&pair[0])?;
        if !matches!(option.as_str(), "IFormula" | "DFormula") {
            return Err(format!("{name}: unknown option '{option}'"));
        }
        let value = text(&pair[1])?;
        if !value.eq_ignore_ascii_case(FORWARD_EULER) {
            return Err(format!(
                "{name}: {option} '{value}' is not supported yet; only 'ForwardEuler' is available"
            ));
        }
    }
    let p = Pid {
        standard,
        gains,
        ts,
    };
    pid_check(&p)?;
    pid_encode(e, &p, char_array(e, "")?, char_array(e, "")?)
}
/// Five significant digits, like MATLAB's `format short g`.
fn pid_number(x: f64) -> String {
    if x == 0. || !x.is_finite() {
        return format!("{x}");
    }
    let exponent = x.abs().log10().floor() as i32;
    if (-5..5).contains(&exponent) {
        let decimals = (4 - exponent).max(0) as usize;
        let s = format!("{x:.decimals$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').into()
        } else {
            s
        }
    } else {
        format!("{x:.4e}")
    }
}
fn pid_display(p: &Pid) -> String {
    let discrete = p.ts > 0.;
    let integrator = if discrete { "Ts/(z-1)" } else { "1/s" };
    let [kp, ki, kd, tf] = pid_parallel(p);
    let mut letters = String::new();
    for (on, l) in [
        (kp != 0., 'P'),
        (ki != 0., 'I'),
        (kd != 0., 'D'),
        (kd != 0. && tf != 0., 'F'),
    ] {
        if on {
            letters.push(l);
        }
    }
    let mut terms = Vec::new();
    let mut with = Vec::new();
    if p.standard {
        let [kp, ti, td, n] = p.gains;
        let mut inner = vec!["1".to_string()];
        with.push(format!("Kp = {}", pid_number(kp)));
        if ti.is_finite() {
            inner.push(format!("1/Ti * {integrator}"));
            with.push(format!("Ti = {}", pid_number(ti)));
        }
        if td != 0. {
            inner.push(match (discrete, n.is_finite()) {
                (false, true) => "Td * s/((Td/N)*s+1)".into(),
                (false, false) => "Td * s".into(),
                (true, true) => "Td * 1/((Td/N)+Ts/(z-1))".into(),
                (true, false) => "Td * (z-1)/Ts".into(),
            });
            with.push(format!("Td = {}", pid_number(td)));
            if n.is_finite() {
                with.push(format!("N = {}", pid_number(n)));
            }
        }
        terms.push(format!("Kp * ({})", inner.join(" + ")));
    } else {
        if kp != 0. || letters.is_empty() {
            terms.push("Kp".into());
            with.push(format!("Kp = {}", pid_number(kp)));
        }
        if ki != 0. {
            terms.push(format!("Ki * {integrator}"));
            with.push(format!("Ki = {}", pid_number(ki)));
        }
        if kd != 0. {
            terms.push(
                match (discrete, tf != 0.) {
                    (false, true) => "Kd * s/(Tf*s+1)",
                    (false, false) => "Kd * s",
                    (true, true) => "Kd * 1/(Tf+Ts/(z-1))",
                    (true, false) => "Kd * (z-1)/Ts",
                }
                .into(),
            );
            with.push(format!("Kd = {}", pid_number(kd)));
            if tf != 0. {
                with.push(format!("Tf = {}", pid_number(tf)));
            }
        }
    }
    let kind = match letters.as_str() {
        "" | "P" => "P-only".to_string(),
        "I" | "D" => format!("{letters}-only"),
        _ => letters,
    };
    let mut body = if kind == "P-only" && !p.standard {
        format!("  Kp = {}\n", pid_number(kp))
    } else {
        format!("  {}\n\n  with {}\n", terms.join(" + "), with.join(", "))
    };
    if discrete {
        body.push_str(&format!("\n  Sample time: {} seconds", pid_number(p.ts)));
    }
    body.push_str(&format!(
        "\n{}-time {kind} controller in {} form.",
        if discrete { "Discrete" } else { "Continuous" },
        if p.standard { "standard" } else { "parallel" }
    ));
    body
}

impl Engine {
    pub(super) fn ctl_set(
        &mut self,
        a: &Array,
        steps: &[PathStep],
        value: &Array,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<Option<Array>> {
        if class(a).is_none() {
            return Ok(None);
        }
        if let [PathStep::Paren(args), rest @ ..] = steps {
            if !rest.is_empty() || deletion {
                return Err(
                    "LTI channel assignment requires a SISO model on the right-hand side".into(),
                );
            }
            return mimo::assign(self, a, args, value, out).map(Some);
        }
        let facade = property_facade(self, a)?;
        let s = &facade;
        let (name, rest) = match steps.split_first() {
            Some((PathStep::Field(n), r)) => (n.clone(), r),
            Some((PathStep::Dynamic(expr), r)) => {
                let v = self.eval(expr, None, out)?;
                (text(&v)?, r)
            }
            _ => {
                return Err(
                    "LTI object indexing/arrays are not supported; assign a named property".into(),
                );
            }
        };
        if private_property(&name) || s.field_index(&name).is_none() {
            return Err(format!("Unknown or private LTI property '{name}'"));
        }
        let old = field(s, &name)?;
        let replacement = self.assign_step(Some(old), rest, value.clone(), deletion, out)?;
        if pid_kind(a).is_some() {
            let mut updated = s.with_field(&name, replacement, &self.cancel)?;
            if name == "Ts" {
                // The forward-Euler formulas follow the sampling interval.
                let ts = scalar(&field(&updated, "Ts")?.into())?;
                let formula = char_array(self, if ts > 0. { FORWARD_EULER } else { "" })?;
                updated = updated
                    .with_field("IFormula", formula.clone(), &self.cancel)?
                    .with_field("DFormula", formula, &self.cancel)?;
            }
            let result = Array::Struct(updated);
            let (_, input, output) = pid_decode(&result)?.ok_or("Invalid PID object")?;
            names_check(&input, 1)?;
            names_check(&output, 1)?;
            return Ok(Some(result));
        }
        Ok(Some(
            encode(self, &replace_properties(self, s, &[(name, replacement)])?)?
                .array(&self.cancel)?,
        ))
    }
    fn ctl_roots(&mut self, p: &Matrix, out: &mut Execution) -> Result<Vec<C>> {
        let v = self.call("roots", vec![Value::Matrix(p.clone())], out)?;
        roots_input(&v)
    }
    fn ctl_poly(&mut self, a: &Matrix, out: &mut Execution) -> Result<Matrix> {
        if a.rows() > MAX_DEGREE {
            return Err("Conversion to polynomial form is limited to 256 states".into());
        }
        real(&self.call("poly", vec![Value::Matrix(a.clone())], out)?)
    }
    pub(super) fn ctl_tf(&mut self, s: &Lti, out: &mut Execution) -> Result<(Matrix, Matrix)> {
        s.require_siso("tf coefficient vectors")?;
        if s.has_internal_delays() {
            return Err(
                "Transfer functions cannot represent internal delays; use pade(sys,N) first".into(),
            );
        }
        match &s.model {
            Model::Tf(n, d) => {
                let Model::Tf(n, d) = normalize(n.clone(), d.clone())? else {
                    unreachable!()
                };
                Ok((n, d))
            }
            Model::Zpk(_, _, k) if *k == 0. => Ok((Matrix::scalar(0.), Matrix::scalar(1.))),
            Model::Zpk(z, p, k) => Ok((poly_roots(z)?.map(|x| x * k), poly_roots(p)?)),
            Model::Ss(a, b, c, d) => {
                if a.rows() == 0 {
                    return Ok((d.clone(), Matrix::scalar(1.)));
                }
                // Expand C(sI-A)^-1 B at infinity using Markov parameters
                // C A^k B, then multiply by the characteristic polynomial.
                // This preserves exact leading structural zeros and avoids
                // subtracting two nearly equal characteristic polynomials.
                let den = self.ctl_poly(a, out)?;
                let n = a.rows();
                let mut num = den.map(|x| x * d.data()[0]).data().to_vec();
                let mut v = b.clone();
                for k in 0..n {
                    self.check_cancel()?;
                    let markov = c.mul(&v)?.as_scalar()?;
                    for (value, coefficient) in num[k + 1..].iter_mut().zip(den.data()) {
                        *value += coefficient * markov;
                    }
                    if k + 1 < n {
                        v = a.mul(&v)?;
                    }
                }
                Ok((coefficients(&Value::Matrix(row(num)?))?, den))
            }
            _ => unreachable!("SISO model required"),
        }
    }
    pub(super) fn ctl_ss(
        &mut self,
        s: &Lti,
        out: &mut Execution,
    ) -> Result<(Matrix, Matrix, Matrix, Matrix)> {
        if let Model::Ss(a, b, c, d) = &s.model {
            return Ok((a.clone(), b.clone(), c.clone(), d.clone()));
        }
        if s.rational_grid() {
            return mimo::realize(self, s, out);
        }
        let (num, den) = self.ctl_tf(s, out)?;
        let (d, coefficients) = companion_coefficients(&num, &den)?;
        let n = den.len() - 1;
        let mut a = vec![0.; n * n];
        let mut b = vec![0.; n];
        let mut c = vec![0.; n];
        for (j, (a_top, output)) in coefficients.enumerate() {
            a[j * n] = a_top;
            c[j] = output;
        }
        for i in 1..n {
            a[i + (i - 1) * n] = 1.;
        }
        if n > 0 {
            b[0] = 1.;
        }
        Ok((
            Matrix::from_column_vec(n, n, a)?,
            Matrix::from_column_vec(n, 1, b)?,
            Matrix::from_column_vec(1, n, c)?,
            Matrix::scalar(d),
        ))
    }
    fn ctl_convert(&mut self, s: &Lti, kind: &str, out: &mut Execution) -> Result<Lti> {
        if s.kind() == kind {
            return Ok(s.clone());
        }
        let model = match kind {
            "ss" => {
                let (a, b, c, d) = self.ctl_ss(s, out)?;
                Model::Ss(a, b, c, d)
            }
            _ if s.ports() != (1, 1) => mimo::convert(self, s, kind, out)?,
            "tf" => {
                let (n, d) = self.ctl_tf(s, out)?;
                normalize(n, d)?
            }
            _ => {
                let (n, d) = self.ctl_tf(s, out)?;
                Model::Zpk(
                    self.ctl_roots(&n, out)?,
                    self.ctl_roots(&d, out)?,
                    n.data()[0] / d.data()[0],
                )
            }
        };
        let mut converted = Lti { model, ..s.clone() };
        converted.props.state = None;
        Ok(converted)
    }
    fn ctl_constructor(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Lti> {
        if args.is_empty() || matches!(args, [Value::Matrix(m)] if m.is_empty() && !m.is_nd()) {
            let model = if name == "ss" {
                let m = Matrix::filled(0, 0, 0.)?;
                ss_model(m.clone(), m.clone(), m.clone(), m)?
            } else {
                mimo::empty_rational(self, name, (0, 0))?
            };
            return new(self, model, 0.);
        }
        if let Some(v) = args.first().filter(|v| is_lti(v)) {
            let sys = self.ctl_convert(&get_lti(self, v)?, name, out)?;
            return apply_options(self, name, sys, &args[1..]);
        }
        if let Some(variable) = args.first().and_then(|v| text(v).ok())
            && matches!(name, "tf" | "zpk")
        {
            let has_ts = args.get(1).is_some_and(|v| matches!(v, Value::Matrix(_)));
            let ts = if has_ts { scalar(&args[1])? } else { 0. };
            if !matches!(variable.as_str(), "s" | "z")
                || (variable == "s" && has_ts)
                || (variable == "z" && (!has_ts || ts == 0.))
            {
                return Err("Use tf('s'), zpk('s'), or tf('z',Ts)".into());
            }
            let model = if name == "tf" {
                Model::Tf(row(vec![1., 0.])?, Matrix::scalar(1.))
            } else {
                Model::Zpk(vec![C::new(0., 0.)], vec![], 1.)
            };
            return apply_options(
                self,
                name,
                new(self, model, ts)?,
                &args[1 + usize::from(has_ts)..],
            );
        }
        if matches!(name, "tf" | "ss")
            && let Some(first) = args.first()
            && args.get(1).is_none_or(|v| text(v).is_ok())
        {
            let sys = interconnect::operand(self, first, name == "ss")?;
            let sys = self.ctl_convert(&sys, name, out)?;
            return apply_options(self, name, sys, &args[1..]);
        }
        let count = match name {
            "tf" => 2,
            "zpk" => 3,
            _ => 4,
        };
        if args.len() < count {
            return Err(format!(
                "{name}: expected {count} model arguments and optional Ts, or one LTI model"
            ));
        }
        let has_ts = args
            .get(count)
            .is_some_and(|v| matches!(v, Value::Matrix(_)));
        let ts = if has_ts { scalar(&args[count])? } else { 0. };
        let options = &args[count + usize::from(has_ts)..];
        let variable = if name == "tf" {
            options
                .as_chunks::<2>()
                .0
                .iter()
                .rev()
                .find_map(|[key, value]| {
                    let key = text(key).ok()?.to_ascii_lowercase();
                    (!key.is_empty() && "variable".starts_with(&key)).then_some(value)
                })
                .map(text)
                .transpose()?
        } else {
            None
        };
        let model = match name {
            "tf" => mimo::tf_model(
                self,
                &args[0],
                &args[1],
                variable.as_deref().unwrap_or("s"),
                false,
            )?,
            "zpk" => mimo::zpk_model(self, &args[0], &args[1], &args[2], false)?,
            _ => {
                let a = real(&args[0])?;
                let b = real(&args[1])?;
                let c = real(&args[2])?;
                let d = real(&args[3])?;
                ss_model(a, b, c, d)?
            }
        };
        let mut sys = new(self, model, ts)?;
        if let Some(variable) = variable {
            sys.variable = variable;
        }
        // Numeric TF coefficients use the requested convention from the start.
        apply_options(self, name, sys, options)
    }
}

/// Controllable companion first row, C entries, and direct term.
fn companion_coefficients<'a>(
    num: &'a Matrix,
    den: &'a Matrix,
) -> Result<(f64, impl Iterator<Item = (f64, f64)> + 'a)> {
    let pad = den
        .len()
        .checked_sub(num.len())
        .ok_or("ss: an improper transfer function has no ordinary state-space realization")?;
    let lead = den.data()[0];
    let feedthrough = if pad == 0 { num.data()[0] / lead } else { 0. };
    let coefficients = den.data()[1..].iter().enumerate().map(move |(j, &v)| {
        let denominator = v / lead;
        let numerator = (j + 1)
            .checked_sub(pad)
            .map_or(0., |k| num.data()[k] / lead);
        (-denominator, numerator - feedthrough * denominator)
    });
    Ok((feedthrough, coefficients))
}
fn ss_model(a: Matrix, mut b: Matrix, mut c: Matrix, mut d: Matrix) -> Result<Model> {
    // Infer explicitly empty B/C dimensions from the state and feedthrough shapes.
    if b.rows() == 0 && b.cols() == 0 {
        b = Matrix::filled(a.rows(), if a.rows() == 0 { d.cols() } else { 0 }, 0.)?;
    }
    if c.rows() == 0 && c.cols() == 0 {
        c = Matrix::filled(if a.rows() == 0 { d.rows() } else { 0 }, a.rows(), 0.)?;
    }
    if d.rows() == 0 && d.cols() == 0 && (c.rows() == 0 || b.cols() == 0) {
        d = Matrix::filled(c.rows(), b.cols(), 0.)?;
    }
    if d.len() == 1 && d.data()[0] == 0. && (c.rows() != 1 || b.cols() != 1) {
        if c.rows().saturating_mul(b.cols()) > MAX_ENTRIES {
            return Err("ss D exceeds its matrix size bound".into());
        }
        d = Matrix::filled(c.rows(), b.cols(), 0.)?;
    }
    ss_check(&a, &b, &c, &d)?;
    Ok(Model::Ss(a, b, c, d))
}
fn block(rows: Vec<Vec<Matrix>>) -> Result<Matrix> {
    Matrix::concatenate(rows)
}
fn neg_model(m: &Model) -> Model {
    match m {
        Model::Tf(n, d) => Model::Tf(n.map(|x| -x), d.clone()),
        Model::Zpk(z, p, k) => Model::Zpk(z.clone(), p.clone(), -k),
        Model::Ss(a, b, c, d) => Model::Ss(a.clone(), b.clone(), c.map(|x| -x), d.map(|x| -x)),
        _ => unreachable!("scalar rational model required"),
    }
}
pub(super) fn negate(e: &Engine, v: &Value) -> Result<Value> {
    encode(e, &interconnect::scale(e, get_lti(e, v)?, -1.)?)
}
fn static_model(s: &Lti) -> bool {
    if s.has_internal_delays() {
        return false;
    }
    match &s.model {
        Model::Tf(n, d) => n.len() == 1 && d.len() == 1,
        Model::Zpk(z, p, _) => z.is_empty() && p.is_empty(),
        Model::Ss(a, _, _, _) => a.rows() == 0,
        Model::TfArray(n, d) => [n, d]
            .into_iter()
            .all(|c| (0..c.len()).all(|i| c.get_linear(i).is_ok_and(|a| a.len() == 1))),
        Model::ZpkArray(z, p, _) => [z, p]
            .into_iter()
            .all(|c| (0..c.len()).all(|i| c.get_linear(i).is_ok_and(|a| a.is_empty()))),
    }
}
fn timebase(a: &Lti, b: &Lti) -> Result<f64> {
    if a.ts == b.ts {
        Ok(a.ts)
    } else if (a.ts == -1. && b.ts > 0.) || (static_model(a) && a.delay() == 0.) {
        Ok(b.ts)
    } else if (b.ts == -1. && a.ts > 0.) || (static_model(b) && b.delay() == 0.) {
        Ok(a.ts)
    } else {
        Err("LTI sampling intervals must agree".into())
    }
}
fn combine_delays(op: &Token, a: &Lti, b: &Lti) -> Result<[f64; 3]> {
    let [ai, ao, aio] = a.props.delay;
    let [bi, bo, bio] = b.props.delay;
    match op {
        // A delay-free gain adds no states. Keep each delay on its plant side.
        Token::Star if a.delay() == 0. && static_model(a) => Ok(b.props.delay),
        Token::Star if b.delay() == 0. && static_model(b) => Ok(a.props.delay),
        Token::Star => Ok([bi, ao, ai + aio + bo + bio]),
        Token::Slash if a.delay() >= b.delay() => Ok([0., ao.min(a.delay() - b.delay()), (a.delay() - b.delay() - ao).max(0.)]),
        Token::Slash => Err("The quotient would have a time advance; use pade(sys,N) before division".into()),
        Token::Plus | Token::Minus if a.delay() == b.delay() => {
            let input = ai.min(bi);
            let output = ao.min(bo);
            Ok([input, output, (a.delay() - input - output).max(0.)])
        }
        _ => Err("Parallel models with different total delays require internal delay states; use pade(sys,N) before combining".into()),
    }
}
fn ss_inverse(a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix) -> Result<Model> {
    let k = d.data()[0];
    if k == 0. {
        return Err(
            "The inverse is improper and cannot be represented by an ordinary ss object".into(),
        );
    }
    ss_model(
        a.coordinate_subtract(&b.mul(c)?.map(|x| x / k))?,
        b.map(|x| x / k),
        c.map(|x| -x / k),
        Matrix::scalar(1. / k),
    )
}
impl Engine {
    fn ctl_combine(&mut self, op: &Token, a: &Lti, b: &Lti, out: &mut Execution) -> Result<Lti> {
        self.check_cancel()?;
        if a.ports() != (1, 1) || b.ports() != (1, 1) {
            return interconnect::combine(self, op, a, b, out);
        }
        let ts = timebase(a, b)?;
        if a.has_internal_delays()
            || b.has_internal_delays()
            || matches!(op, Token::Plus | Token::Minus) && a.delay() != b.delay()
        {
            return delays::interconnect(self, op, a, b, ts, out);
        }
        let delays = combine_delays(op, a, b)?;
        let kind = if rank(a.kind()) >= rank(b.kind()) {
            a.kind()
        } else {
            b.kind()
        };
        let model = if kind == "ss" {
            let (a1, b1, c1, d1) = self.ctl_ss(a, out)?;
            let mut right = self.ctl_convert(b, "ss", out)?;
            if *op == Token::Minus {
                right.model = neg_model(&right.model);
            }
            if *op == Token::Slash {
                let Model::Ss(aa, bb, cc, dd) = &right.model else {
                    unreachable!()
                };
                right.model = ss_inverse(aa, bb, cc, dd)?;
            }
            let Model::Ss(a2, b2, c2, d2) = right.model else {
                unreachable!()
            };
            let n1 = a1.rows();
            let n2 = a2.rows();
            if n1 + n2 > MAX_STATES {
                return Err("Interconnection exceeds 1024 states".into());
            }
            match op {
                Token::Plus | Token::Minus => ss_model(
                    block(vec![
                        vec![a1, Matrix::filled(n1, n2, 0.)?],
                        vec![Matrix::filled(n2, n1, 0.)?, a2],
                    ])?,
                    block(vec![vec![b1], vec![b2]])?,
                    block(vec![vec![c1, c2]])?,
                    d1.coordinate_add(&d2)?,
                )?,
                Token::Star | Token::Slash => ss_model(
                    block(vec![
                        vec![a1, b1.mul(&c2)?],
                        vec![Matrix::filled(n2, n1, 0.)?, a2],
                    ])?,
                    block(vec![vec![b1.mul(&d2)?], vec![b2]])?,
                    block(vec![vec![c1, d1.mul(&c2)?]])?,
                    d1.mul(&d2)?,
                )?,
                _ => return Err("Unsupported LTI operator".into()),
            }
        } else if kind == "zpk" && matches!(op, Token::Star | Token::Slash) {
            let left = self.ctl_convert(a, "zpk", out)?;
            let right = self.ctl_convert(b, "zpk", out)?;
            let Model::Zpk(mut z, mut p, k) = left.model else {
                unreachable!()
            };
            let Model::Zpk(z2, p2, k2) = right.model else {
                unreachable!()
            };
            let gain = if *op == Token::Slash {
                if k2 == 0. {
                    return Err("Cannot divide by a zero LTI model".into());
                }
                z.extend(p2);
                p.extend(z2);
                k / k2
            } else {
                z.extend(z2);
                p.extend(p2);
                k * k2
            };
            if z.len() > MAX_DEGREE || p.len() > MAX_DEGREE || !gain.is_finite() {
                return Err(
                    "ZPK interconnection exceeds 256 roots or its finite gain bound".into(),
                );
            }
            poly_roots(&z)?;
            poly_roots(&p)?;
            Model::Zpk(z, p, gain)
        } else {
            let (n1, d1) = self.ctl_tf(a, out)?;
            let (n2, d2) = self.ctl_tf(b, out)?;
            let (num, den) = match op {
                Token::Plus | Token::Minus => (
                    addpoly(
                        &conv(n1.data(), d2.data())?,
                        &conv(n2.data(), d1.data())?,
                        if *op == Token::Minus { -1. } else { 1. },
                    ),
                    conv(d1.data(), d2.data())?,
                ),
                Token::Star => (conv(n1.data(), n2.data())?, conv(d1.data(), d2.data())?),
                Token::Slash => {
                    if n2.data().iter().all(|x| *x == 0.) {
                        return Err("Cannot divide by a zero LTI model".into());
                    }
                    (conv(n1.data(), d2.data())?, conv(d1.data(), n2.data())?)
                }
                _ => return Err("Unsupported LTI operator".into()),
            };
            normalize(
                coefficients(&Value::Matrix(row(num)?))?,
                coefficients(&Value::Matrix(row(den)?))?,
            )?
        };
        let mut result = new(self, model, ts)?;
        result.input = match op {
            Token::Star => b.input.clone(),
            Token::Slash => b.output.clone(),
            _ => a.input.clone(),
        };
        result.output = a.output.clone();
        result.props.delay = delays;
        copy_metadata(&mut result, a, b);
        delay_check(&result)?;
        let mut result = self.ctl_convert(&result, kind, out)?;
        if kind == "ss" {
            result.props.state = joined_states(self, [a, b])?;
        }
        Ok(result)
    }
    pub(super) fn ctl_binary(
        &mut self,
        op: &Token,
        left: &Value,
        right: &Value,
        out: &mut Execution,
    ) -> Result<Value> {
        if *op == Token::Power {
            if !is_lti(left) || is_lti(right) {
                return Err("LTI powers require an LTI base and integer scalar exponent".into());
            }
            let exponent = scalar(right)?;
            if !exponent.is_finite() || exponent.fract() != 0. || exponent.abs() > 64. {
                return Err("LTI exponent must be an integer in -64..64".into());
            }
            let mut base = get_lti(self, left)?;
            base.require_siso("^")?;
            let mut result = new(
                self,
                Model::Tf(Matrix::scalar(1.), Matrix::scalar(1.)),
                base.ts,
            )?;
            result = self.ctl_convert(&result, base.kind(), out)?;
            let mut n = exponent.abs() as u32;
            if exponent < 0. {
                base = self.ctl_combine(&Token::Slash, &result, &base, out)?;
            }
            while n > 0 {
                self.check_cancel()?;
                if n & 1 != 0 {
                    result = self.ctl_combine(&Token::Star, &result, &base, out)?;
                }
                n >>= 1;
                if n > 0 {
                    base = self.ctl_combine(&Token::Star, &base, &base, out)?;
                }
            }
            return encode(self, &result);
        }
        if !matches!(op, Token::Plus | Token::Minus | Token::Star | Token::Slash) {
            return Err("LTI operators support +, -, *, /, and integer ^".into());
        }
        let s = interconnect::binary(self, op, left, right, out)?;
        encode(self, &s)
    }
    fn ctl_feedback(&mut self, g: &Lti, h: &Lti, sign: f64, out: &mut Execution) -> Result<Lti> {
        g.require_siso("feedback")?;
        h.require_siso("feedback")?;
        let ts = timebase(g, h)?;
        if g.delay() != 0. || h.delay() != 0. || g.has_internal_delays() || h.has_internal_delays()
        {
            return Err("Feedback with transport delays requires internal delays; use pade(sys,N) on continuous-time models before feedback".into());
        }
        if sign != 1. && sign != -1. {
            return Err("feedback sign must be +1 or -1".into());
        }
        let kind = if rank(g.kind()) >= rank(h.kind()) {
            g.kind()
        } else {
            h.kind()
        };
        let model = if kind == "ss" {
            let (a, b, c, d) = self.ctl_ss(g, out)?;
            let (ah, bh, ch, dh) = self.ctl_ss(h, out)?;
            let f = 1. - sign * d.data()[0] * dh.data()[0];
            if f == 0. || !f.is_finite() {
                return Err(interconnect::LOOP.into());
            }
            let ng = a.rows();
            let nh = ah.rows();
            if ng + nh > MAX_STATES {
                return Err("Feedback exceeds 1024 states".into());
            }
            let dg = d.data()[0];
            let hh = dh.data()[0];
            ss_model(
                block(vec![
                    vec![
                        a.coordinate_add(&b.mul(&c)?.map(|x| x * sign * hh / f))?,
                        b.mul(&ch)?.map(|x| x * sign / f),
                    ],
                    vec![
                        bh.mul(&c)?.map(|x| x / f),
                        ah.coordinate_add(&bh.mul(&ch)?.map(|x| x * sign * dg / f))?,
                    ],
                ])?,
                block(vec![vec![b.map(|x| x / f)], vec![bh.map(|x| x * dg / f)]])?,
                block(vec![vec![c.map(|x| x / f), ch.map(|x| x * sign * dg / f)]])?,
                Matrix::scalar(dg / f),
            )?
        } else {
            let (ng, dg) = self.ctl_tf(g, out)?;
            let (nh, dh) = self.ctl_tf(h, out)?;
            if ng.len() == dg.len()
                && nh.len() == dh.len()
                && 1. - sign * (ng.data()[0] / dg.data()[0]) * (nh.data()[0] / dh.data()[0]) == 0.
            {
                return Err(interconnect::LOOP.into());
            }
            normalize(
                coefficients(&Value::Matrix(row(conv(ng.data(), dh.data())?)?))?,
                coefficients(&Value::Matrix(row(addpoly(
                    &conv(dg.data(), dh.data())?,
                    &conv(ng.data(), nh.data())?,
                    -sign,
                ))?))?,
            )?
        };
        let mut s = new(self, model, ts)?;
        s.input = g.input.clone();
        s.output = g.output.clone();
        copy_metadata(&mut s, g, h);
        let mut s = self.ctl_convert(&s, kind, out)?;
        if kind == "ss" {
            s.props.state = joined_states(self, [g, h])?;
        }
        Ok(s)
    }
    pub(super) fn ctl_poles(&mut self, s: &Lti, out: &mut Execution) -> Result<Vec<C>> {
        match &s.model {
            Model::Tf(_, d) => {
                // Use real division before roots forms complex coefficient ratios.
                let lead = d.data()[0];
                if lead == 1. {
                    return self.ctl_roots(d, out);
                }
                let monic = d.map(|x| x / lead);
                if monic.data().iter().any(|x| !x.is_finite()) {
                    return Err(
                        "roots: coefficients overflow when scaled by the leading term".into(),
                    );
                }
                self.ctl_roots(&monic, out)
            }
            Model::Zpk(_, p, _) => Ok(p.clone()),
            Model::TfArray(..) | Model::ZpkArray(..) => mimo::poles(self, s, out),
            Model::Ss(a, _, _, _) => {
                if a.rows() == 0 {
                    return Ok(vec![]);
                }
                let v = self.call("eig", vec![Value::Matrix(a.clone())], out)?;
                // State matrices may exceed the polynomial/root conversion bound.
                match v {
                    Value::Complex(m) => Ok(m.data().to_vec()),
                    Value::Matrix(m) => Ok(m.data().iter().map(|&x| C::new(x, 0.)).collect()),
                    _ => Err("Invalid eigenvalue result".into()),
                }
            }
        }
    }
    fn ctl_minreal(&mut self, s: &Lti, tol: f64, out: &mut Execution) -> Result<Lti> {
        if !tol.is_finite() || tol < 0. {
            return Err("minreal tolerance must be finite and nonnegative".into());
        }
        if s.ports() != (1, 1) {
            return mimo::minreal(self, s, tol, out);
        }
        let (z, p, gain) = if let Model::Zpk(z, p, k) = &s.model {
            (z.clone(), p.clone(), *k)
        } else {
            let (n, d) = self.ctl_tf(s, out)?;
            (
                self.ctl_roots(&n, out)?,
                self.ctl_roots(&d, out)?,
                n.data()[0] / d.data()[0],
            )
        };
        let z = cancellation_groups(self, z, tol)?;
        let mut p = cancellation_groups(self, p, tol)?;
        let mut kept = Vec::new();
        for (root, conjugate) in z {
            self.check_cancel()?;
            let near = p
                .iter()
                .enumerate()
                .filter_map(|(i, (pole, partner))| {
                    if (root.im == 0.) != (pole.im == 0.) {
                        return None;
                    }
                    let distance = (*pole - root).norm();
                    if distance > tol * (1. + root.norm()) {
                        return None;
                    }
                    match (conjugate, partner) {
                        (None, None) => Some((i, distance)),
                        (Some(z), Some(p)) if (*p - z).norm() <= tol * (1. + z.norm()) => {
                            Some((i, distance.max((*p - z).norm())))
                        }
                        _ => None,
                    }
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, _)) = near {
                p.remove(i);
            } else {
                kept.push(root);
                kept.extend(conjugate);
            }
        }
        let p = p
            .into_iter()
            .flat_map(|(root, partner)| std::iter::once(root).chain(partner))
            .collect::<Vec<_>>();
        // Zero transfer functions have no observable dynamics.
        let model = if gain == 0. {
            Model::Tf(Matrix::scalar(0.), Matrix::scalar(1.))
        } else if s.kind() == "zpk" {
            Model::Zpk(kept, p, gain)
        } else {
            normalize(poly_roots(&kept)?.map(|x| x * gain), poly_roots(&p)?)?
        };
        let mut reduced = Lti { model, ..s.clone() };
        reduced.props.state = None;
        self.ctl_convert(&reduced, s.kind(), out)
    }
    fn ctl_matrix_function(&self, name: &str, m: Matrix) -> Result<Matrix> {
        let mut values = self
            .linalg2_call(name, &[Value::Matrix(m)], 1)?
            .ok_or("Matrix function unavailable")?;
        match values.remove(0) {
            Value::Matrix(m) if m.data().iter().all(|x| x.is_finite()) => Ok(m),
            Value::Complex(m) => {
                let scale = m.data().iter().map(|x| x.norm()).fold(1., f64::max);
                if m.data()
                    .iter()
                    .any(|x| !x.re.is_finite() || !x.im.is_finite() || x.im.abs() > 1e-9 * scale)
                {
                    return Err("d2c: no real principal logarithm for this sampled system".into());
                }
                m.real(&self.cancel)
            }
            _ => Err("Sampling transformation produced a nonfinite matrix".into()),
        }
    }
    pub(super) fn ctl_sample(
        &mut self,
        s: &Lti,
        ts: f64,
        method: &str,
        inverse: bool,
        out: &mut Execution,
    ) -> Result<Lti> {
        if !ts.is_finite() || ts <= 0. {
            return Err("Sampling interval must be positive and finite".into());
        }
        if inverse && s.ts == 0. {
            return Err("d2c requires a discrete-time model".into());
        }
        if !inverse && s.ts != 0. {
            return Err("c2d requires a continuous-time model".into());
        }
        if !matches!(method, "zoh" | "tustin" | "foh") || (inverse && method == "foh") {
            return Err(
                "Use c2d methods 'zoh', 'tustin', 'foh', or d2c methods 'zoh', 'tustin'".into(),
            );
        }
        if s.has_internal_delays() {
            let mut paths = delays::full_paths(s)
                .iter()
                .map(|path| self.ctl_sample(path, ts, method, inverse, out))
                .collect::<Result<Vec<_>>>()?;
            // State coordinates and their ordering survive the per-path transform.
            if let Some(Array::Cell(names)) = &s.props.state {
                let mut offset = 0;
                for path in &mut paths {
                    let n = path.states();
                    let labels = (offset..offset + n)
                        .map(|i| names.get_linear(i).cloned())
                        .collect::<Result<Vec<_>>>()?;
                    path.props.state =
                        Some(CellArray::from_shared(n, 1, labels, &self.cancel)?.into());
                    offset += n;
                }
            }
            let mut result = delays::direct_sum(self, paths, if inverse { 0. } else { ts })?;
            delays::keep_outer_properties(&mut result, s);
            return Ok(result);
        }
        let (a, b, c, d) = self.ctl_ss(s, out)?;
        let n = a.rows();
        let m = b.cols();
        let eye = Matrix::identity(n, n)?;
        let model =
            if n == 0 {
                Model::Ss(a, b, c, d)
            } else if method == "tustin" {
                if inverse {
                    // Invert the generalized bilinear state-coordinate convention:
                    // Ac=2/T (Ad+I)\(Ad-I), Bc=2/T (Ad+I)\Bd,
                    // Cc=2 Cd/(Ad+I), Dc=Dd-Cd(Ad+I)\Bd.
                    let plus = a.coordinate_add(&eye)?;
                    let x = plus.solve(&b)?;
                    let ac = plus
                        .solve(&a.coordinate_subtract(&eye)?)?
                        .map(|x| 2. * x / ts);
                    let bc = x.map(|x| 2. * x / ts);
                    let cc = plus
                        .transpose()
                        .solve(&c.transpose())?
                        .transpose()
                        .map(|x| 2. * x);
                    let correction = c.mul(&x)?;
                    let residual = d.coordinate_subtract(&correction)?;
                    // Cancellation in Dc = Dd - Cd*(Ad+I)\Bd can leave a
                    // spurious feedthrough (and a huge artificial finite zero).
                    // Snap only within a componentwise roundoff envelope of the
                    // terms being subtracted, never an absolute gain threshold.
                    let envelope = d
                        .map(f64::abs)
                        .coordinate_add(&c.map(f64::abs).mul(&x.map(f64::abs))?)?;
                    let roundoff = 64. * f64::EPSILON * n.max(1) as f64;
                    let dc = residual.zip(&envelope, |r, scale| {
                        if r.abs() <= roundoff * scale { 0. } else { r }
                    })?;
                    ss_model(ac, bc, cc, dc)?
                } else {
                    let minus = eye.coordinate_subtract(&a.map(|x| x * ts / 2.))?;
                    let ad = minus.solve(&eye.coordinate_add(&a.map(|x| x * ts / 2.))?)?;
                    let x = minus.solve(&b)?;
                    let bd = x.map(|x| x * ts);
                    let cd = minus.transpose().solve(&c.transpose())?.transpose();
                    let dd = d.coordinate_add(&c.mul(&x)?.map(|x| x * ts / 2.))?;
                    ss_model(ad, bd, cd, dd)?
                }
            } else if inverse {
                if n + m > MAX_STATES {
                    return Err("Augmented logarithm exceeds 1024 dimensions".into());
                }
                // log([Ad Bd; 0 I])/Ts = [Ac Bc; 0 0] on the principal branch.
                let augmented = block(vec![
                    vec![a, b],
                    vec![Matrix::filled(m, n, 0.)?, Matrix::identity(m, m)?],
                ])?;
                let log = self.ctl_matrix_function("logm", augmented)?.map(|x| x / ts);
                ss_model(
                    submatrix(&log, 0, 0, n, n)?,
                    submatrix(&log, 0, n, n, m)?,
                    c,
                    d,
                )?
            } else {
                let extra = if method == "foh" { 2 * m } else { m };
                if n + extra > MAX_STATES {
                    return Err("Augmented exponential exceeds 1024 dimensions".into());
                }
                let augmented = if method == "zoh" {
                    block(vec![
                        vec![a.map(|x| x * ts), b.map(|x| x * ts)],
                        vec![Matrix::filled(m, n, 0.)?, Matrix::filled(m, m, 0.)?],
                    ])?
                } else {
                    // exp([A*T B*T 0; 0 0 I; 0 0 0]) integrates a linear ramp.
                    block(vec![
                        vec![
                            a.map(|x| x * ts),
                            b.map(|x| x * ts),
                            Matrix::filled(n, m, 0.)?,
                        ],
                        vec![
                            Matrix::filled(m, n, 0.)?,
                            Matrix::filled(m, m, 0.)?,
                            Matrix::identity(m, m)?,
                        ],
                        vec![
                            Matrix::filled(m, n, 0.)?,
                            Matrix::filled(m, m, 0.)?,
                            Matrix::filled(m, m, 0.)?,
                        ],
                    ])?
                };
                let exp = self.ctl_matrix_function("expm", augmented)?;
                let ad = submatrix(&exp, 0, 0, n, n)?;
                let b0 = submatrix(&exp, 0, n, n, m)?;
                if method == "zoh" {
                    ss_model(ad, b0, c, d)?
                } else {
                    let b1 = submatrix(&exp, 0, n + m, n, m)?;
                    let bd = b0.coordinate_subtract(&b1)?.coordinate_add(&ad.mul(&b1)?)?;
                    let dd = d.coordinate_add(&c.mul(&b1)?)?;
                    ss_model(ad, bd, c, dd)?
                }
            };
        let mut sampled = Lti {
            model,
            ts: if inverse { 0. } else { ts },
            variable: if inverse { "s" } else { "z" }.into(),
            ..s.clone()
        };
        for (index, delay) in s.props.delay.into_iter().enumerate() {
            sampled.props.delay[index] = if inverse {
                delay * ts
            } else {
                let samples = delay / ts;
                let nearest = samples.round();
                if !samples.is_finite()
                    || (samples - nearest).abs() > (16. * f64::EPSILON * samples).min(0.125)
                {
                    return Err(format!(
                        "c2d: fractional {} is not supported; delays must be integer multiples of the sample time",
                        DELAYS[index]
                    ));
                }
                nearest
            };
        }
        delay_check(&sampled)?;
        self.check_cancel()?;
        self.ctl_convert(&sampled, s.kind(), out)
    }
}
fn submatrix(a: &Matrix, r: usize, c: usize, n: usize, m: usize) -> Result<Matrix> {
    let mut data = Vec::with_capacity(n * m);
    for j in c..c + m {
        for i in r..r + n {
            data.push(a.get(i, j));
        }
    }
    Matrix::from_column_vec(n, m, data)
}

fn lti_data_slot(e: &Engine, a: Array, vector: bool) -> Result<Value> {
    Ok(if vector { a } else { cell(e, a)? }.into())
}
impl Engine {
    pub(super) fn ctl_data(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if name == "pade" {
            return lti_pade(self, args, count, out);
        }
        let max = match name {
            "tfdata" | "damp" => 3,
            "zpkdata" => 4,
            _ => 5,
        };
        if (count == 0 && name != "damp")
            || count > max
            || args.is_empty()
            || args.len()
                > if name == "tfdata" || name == "zpkdata" {
                    2
                } else {
                    1
                }
        {
            return Err(format!("{name}: unsupported argument or output count"));
        }
        if !is_lti(&args[0]) {
            return Err(format!("{name} requires an LTI object"));
        }
        let s = get_lti(self, &args[0])?;
        let vector = if args.len() == 2 {
            if text(&args[1])? != "v" {
                return Err("Use 'v' for vector data instead of cells".into());
            }
            true
        } else {
            false
        };
        let mut values = match name {
            "tfdata" | "zpkdata" => {
                if vector {
                    s.require_siso(name)?;
                }
                let kind = if name == "tfdata" { "tf" } else { "zpk" };
                let converted = self.ctl_convert(&s, kind, out)?;
                let mut data = match converted.model {
                    Model::Tf(n, d) => {
                        let (n, d) = public_tf(&n, &d, &s.variable)?;
                        vec![
                            lti_data_slot(self, n.into(), vector)?,
                            lti_data_slot(self, d.into(), vector)?,
                        ]
                    }
                    Model::Zpk(z, p, k) => {
                        let values =
                            [z, p].map(|r| roots_value(&self.cancel, &r)?.array(&self.cancel));
                        let [z, p] = values;
                        vec![
                            lti_data_slot(self, z?, vector)?,
                            lti_data_slot(self, p?, vector)?,
                            Value::Matrix(Matrix::scalar(k)),
                        ]
                    }
                    Model::TfArray(n, d) => vec![Array::Cell(n).into(), Array::Cell(d).into()],
                    Model::ZpkArray(z, p, k) => vec![
                        Array::Cell(z).into(),
                        Array::Cell(p).into(),
                        Value::Matrix(k),
                    ],
                    _ => unreachable!(),
                };
                data.push(Value::Matrix(Matrix::scalar(s.ts)));
                data
            }
            "ssdata" => {
                // Internal delays have documented zero-delay A/B/C/D matrices.
                // These outputs have no slot for an external transport delay.
                if s.delay() != 0. {
                    s.require_delay_free("ssdata")?;
                }
                let (a, b, c, d) = self.ctl_ss(&s, out)?;
                vec![
                    Value::Matrix(a),
                    Value::Matrix(b),
                    Value::Matrix(c),
                    Value::Matrix(d),
                    Value::Matrix(Matrix::scalar(s.ts)),
                ]
            }
            "damp" => {
                let mut p = self.ctl_poles(&s, out)?;
                let ts = s.sample_period();
                p.sort_by(|a, b| {
                    equivalent_pole(*a, ts)
                        .norm()
                        .total_cmp(&equivalent_pole(*b, ts).norm())
                });
                let equivalent: Vec<_> = p.iter().map(|p| equivalent_pole(*p, ts)).collect();
                let wn: Vec<_> = equivalent.iter().map(|p| p.norm()).collect();
                let zeta: Vec<_> = equivalent
                    .iter()
                    .zip(&wn)
                    .map(|(p, w)| {
                        if w.is_infinite() && p.re == f64::NEG_INFINITY {
                            1.
                        } else {
                            -p.re / w
                        }
                    })
                    .collect();
                if count == 0 {
                    self.emit(out, damp_table(&p, &equivalent, &wn, &zeta, ts != 0.))?;
                    return Ok(Vec::new());
                }
                vec![
                    Value::Matrix(Matrix::from_column_vec(p.len(), 1, wn)?),
                    Value::Matrix(Matrix::from_column_vec(p.len(), 1, zeta)?),
                    roots_value(&self.cancel, &p)?,
                ]
            }
            _ => return Err("Unknown LTI data function".into()),
        };
        values.truncate(count);
        self.check_cancel()?;
        Ok(values)
    }
    pub(super) fn ctl_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        if matches!(name, "tf" | "zpk" | "ss") {
            let sys = self.ctl_constructor(name, args, out)?;
            return encode(self, &sys).map(Some);
        }
        if matches!(name, "pid" | "pidstd") {
            return pid_constructor(self, name, args).map(Some);
        }
        if data_name(name) {
            return self
                .ctl_data(name, args, 1, out)
                .map(|mut v| Some(v.remove(0)));
        }
        let has_lti = args.iter().any(is_lti);
        if !NAMES.contains(&name) && !has_lti {
            return Ok(None);
        }
        if let Some(Value::Container(a)) = args.first()
            && class(a).is_some()
        {
            match name {
                "setfield" | "rmfield" | "struct2cell" => {
                    return Err(format!(
                        "{name} is not supported for LTI values; use validated dot properties"
                    ));
                }
                "get" => {
                    let facade = property_facade(self, a)?;
                    return Ok(Some(match &args[1..] {
                        [] => Value::Container(public_properties(self, facade)?.into()),
                        [key] => field(&facade, &property_name(&facade, &text(key)?)?)?.into(),
                        _ => {
                            return Err(
                                "get: expected an LTI model and optional property name".into()
                            );
                        }
                    }));
                }
                "set" if pid_kind(a).is_none() => {
                    return lti_set_options(self, a, &args[1..]).map(Some);
                }
                "fieldnames" | "isfield" | "numfields" | "getfield" => {
                    let facade = property_facade(self, a)?;
                    let mut public_args = args.to_vec();
                    public_args[0] = Value::Container(public_properties(self, facade)?.into());
                    return self.struct_call(name, &public_args);
                }
                _ => {}
            }
        }
        if matches!(name, "series" | "parallel" | "feedback" | "append") {
            return interconnect::call(self, name, args, out).map(Some);
        }
        if !matches!(
            name,
            "class"
                | "isa"
                | "inv"
                | "disp"
                | "isstruct"
                | "isnumeric"
                | "pole"
                | "zero"
                | "dcgain"
                | "minreal"
                | "c2d"
                | "d2c"
                | "isstable"
                | "order"
                | "issiso"
        ) {
            return Ok(None);
        }
        let (min, max) = match name {
            "isa" => (2, 2),
            "c2d" => (2, usize::MAX),
            "d2c" | "minreal" => (1, 2),
            _ => (1, 1),
        };
        if args.len() < min || args.len() > max || !args.first().is_some_and(is_lti) {
            return Err(format!("{name}: expected an LTI model and valid options"));
        }
        let s = get_lti(self, &args[0])?;
        let kind = match &args[0] {
            Value::Container(a) => class(a).unwrap_or(s.kind()),
            _ => s.kind(),
        };
        let value = match name {
            "class" => Value::Text(kind.into()),
            "isa" => Value::Logical(LogicalMatrix::scalar(
                matches!(text(&args[1])?.as_str(), "lti") || text(&args[1])? == kind,
            )),
            "issiso" => Value::Logical(LogicalMatrix::scalar(s.ports() == (1, 1))),
            "isstruct" | "isnumeric" => Value::Logical(LogicalMatrix::scalar(false)),
            "disp" => {
                let body = display(self, &args[0].clone().array(&self.cancel)?)?
                    .ok_or("Invalid LTI display")?;
                self.emit(out, body)?;
                Value::Void
            }
            "inv" => {
                s.require_siso("inv")?;
                let one = new(
                    self,
                    Model::Tf(Matrix::scalar(1.), Matrix::scalar(1.)),
                    s.ts,
                )?;
                let mut result = self.ctl_combine(&Token::Slash, &one, &s, out)?;
                result.output = s.input.clone();
                encode(self, &result)?
            }
            "pole" => {
                let p = self.ctl_poles(&s, out)?;
                roots_value(&self.cancel, &p)?
            }
            "zero" => {
                s.require_siso("zero")?;
                let z = if let Model::Zpk(z, _, _) = &s.model {
                    z.clone()
                } else {
                    let (n, _) = self.ctl_tf(&s, out)?;
                    self.ctl_roots(&n, out)?
                };
                roots_value(&self.cancel, &z)?
            }
            "dcgain" => {
                if s.ports() == (1, 1) {
                    let mut base = s.clone();
                    base.props.paths.clear();
                    let (n, d) = self.ctl_tf(&base, out)?;
                    let x = if s.ts == 0. { 0. } else { 1. };
                    Value::Matrix(Matrix::scalar(
                        evalpoly(n.data(), x) / evalpoly(d.data(), x),
                    ))
                } else {
                    let (a, b, c, d) = self.ctl_ss(&s, out)?;
                    if a.rows() == 0 {
                        Value::Matrix(d)
                    } else {
                        let lhs = if s.ts == 0. {
                            a.map(|x| -x)
                        } else {
                            Matrix::identity(a.rows(), a.cols())?.coordinate_subtract(&a)?
                        };
                        Value::Matrix(d.coordinate_add(&c.mul(&lhs.solve(&b)?)?)?)
                    }
                }
            }
            "isstable" => {
                let p = self.ctl_poles(&s, out)?;
                Value::Logical(LogicalMatrix::scalar(
                    p.iter()
                        .all(|p| if s.ts == 0. { p.re < 0. } else { p.norm() < 1. }),
                ))
            }
            "order" => Value::Matrix(Matrix::scalar(match &s.model {
                Model::Ss(a, _, _, _) => a.rows(),
                Model::Tf(_, d) => d.len() - 1,
                Model::Zpk(_, p, _) => p.len(),
                _ => mimo::order(self, &s)?,
            } as f64)),
            "minreal" => {
                let result = self.ctl_minreal(
                    &s,
                    if args.len() == 2 {
                        scalar(&args[1])?
                    } else {
                        f64::EPSILON.sqrt()
                    },
                    out,
                )?;
                encode(self, &result)?
            }
            "c2d" => {
                let ts = scalar(&args[1])?;
                let method = args
                    .get(2)
                    .and_then(|v| text(v).ok())
                    .filter(|method| matches!(method.as_str(), "zoh" | "tustin" | "foh"));
                let options = &args[2 + usize::from(method.is_some())..];
                if options.len() == 1 {
                    return Err(
                        "Use c2d methods 'zoh', 'tustin', or 'foh', and property name-value pairs"
                            .into(),
                    );
                }
                let result =
                    self.ctl_sample(&s, ts, method.as_deref().unwrap_or("zoh"), false, out)?;
                encode(self, &apply_options(self, "c2d", result, options)?)?
            }
            "d2c" => {
                let method = if args.len() == 2 {
                    text(&args[1])?
                } else {
                    "zoh".into()
                };
                let result = self.ctl_sample(&s, s.ts, &method, true, out)?;
                encode(self, &result)?
            }
            _ => unreachable!(),
        };
        Ok(Some(value))
    }
}
// Diagonal Padé of exp(-sT). Consecutive denominator coefficients
// of ascending powers have ratio T*(N-k+1)/(k*(2*N-k+1)).
fn pade_coefficients(delay: f64, order: usize) -> Result<(Matrix, Matrix)> {
    if delay == 0. || order == 0 {
        return Ok((Matrix::scalar(1.), Matrix::scalar(1.)));
    }
    let mut den = vec![1.; order + 1];
    for k in (1..=order).rev() {
        let ratio = (k * (2 * order - k + 1)) as f64 / (order - k + 1) as f64;
        den[order - k + 1] = den[order - k] / delay * ratio;
    }
    if den.iter().any(|a| !a.is_finite() || *a == 0.) {
        return Err("Padé coefficients exceed the finite double-precision range".into());
    }
    let num = den
        .iter()
        .enumerate()
        .map(|(i, &a)| if (order - i).is_multiple_of(2) { a } else { -a })
        .collect();
    Ok((row(num)?, row(den)?))
}
fn lti_pade(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let [target, degree] = args else {
        return Err("Use pade(T,N) or pade(sys,N)".into());
    };
    let n = scalar(degree).map_err(|_| "Padé order must be an integer in 0..32")?;
    if !(0. ..=32.).contains(&n) || n.fract() != 0. {
        return Err("Padé order must be an integer in 0..32".into());
    }
    let order = n as usize;
    if is_lti(target) && count == 1 {
        let mut s = get_lti(e, target)?;
        if s.ts != 0. {
            return Err("pade(sys,N) requires a continuous-time LTI model".into());
        }
        s = pade_model(e, s, order, out)?;
        return Ok(vec![encode(e, &s)?]);
    }
    if !(1..=2).contains(&count) || is_lti(target) {
        return Err("pade(T,N) returns up to two outputs; pade(sys,N) returns one model".into());
    }
    let delay = scalar(target).map_err(|_| "Padé delay must be a finite nonnegative scalar")?;
    if !delay.is_finite() || delay < 0. {
        return Err("Padé delay must be a finite nonnegative scalar".into());
    }
    let (n, d) = pade_coefficients(delay, order)?;
    let mut values = vec![Value::Matrix(n), Value::Matrix(d)];
    values.truncate(count);
    Ok(values)
}
fn pade_model(e: &mut Engine, mut s: Lti, order: usize, out: &mut Execution) -> Result<Lti> {
    let [input, output, io] = std::mem::take(&mut s.props.delay);
    let internal = std::mem::take(&mut s.props.paths);
    if order == 0 {
        return Ok(s);
    }
    if !internal.is_empty() {
        let paths = internal
            .into_iter()
            .map(|path| pade_model(e, path, order, out))
            .collect::<Result<Vec<_>>>()?;
        let result = delays::direct_sum(e, paths, s.ts)?;
        s.model = result.model;
        s.props.state = result.props.state;
    }
    for (delay, at_output) in [(input, false), (output, true), (io, false)] {
        if delay == 0. {
            continue;
        }
        let (n, d) = pade_coefficients(delay, order)?;
        let approximation = new(e, Model::Tf(n, d), 0.)?;
        let (left, right) = if at_output {
            (&approximation, &s)
        } else {
            (&s, &approximation)
        };
        let result = e.ctl_combine(&Token::Star, left, right, out)?;
        s.model = result.model;
        s.props.state = result.props.state;
    }
    Ok(s)
}
fn damp_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x.is_sign_negative() { "-Inf" } else { "Inf" }.into();
    }
    let raw = format!("{:.2e}", if x == 0. { 0. } else { x });
    let (mantissa, exponent) = raw.split_once('e').expect("scientific format");
    let exponent: i32 = exponent.parse().expect("scientific exponent");
    format!("{mantissa}e{exponent:+03}")
}
fn damp_table(p: &[C], equivalent: &[C], wn: &[f64], zeta: &[f64], discrete: bool) -> String {
    use std::fmt::Write;
    let mut table = if discrete {
        "         Pole             Magnitude     Damping       Frequency      Time Constant\n                                                    (rad/seconds)      (seconds)".to_owned()
    } else {
        "         Pole              Damping       Frequency      Time Constant\n                                       (rad/seconds)      (seconds)".to_owned()
    };
    table.push('\n');
    let mut indices: Vec<_> = (0..p.len()).collect();
    // Pole computations yield conjugate neighbours; render the upper-half
    // member first while keeping the output vectors in their original order.
    let mut column = 0;
    while column + 1 < indices.len() {
        let (lower, upper) = (p[column], p[column + 1]);
        if lower.im < 0. && upper == lower.conj() {
            indices.swap(column, column + 1);
            column += 2;
        } else {
            column += 1;
        }
    }
    for i in indices {
        let (p, eq, w, z) = (p[i], equivalent[i], wn[i], zeta[i]);
        let mut pole = format!("{:>10}", damp_number(p.re));
        if p.im != 0. {
            let sign = if p.im < 0. { '-' } else { '+' };
            let _ = write!(pole, " {sign} {}i", damp_number(p.im.abs()));
        }
        let _ = write!(table, "\n{pole:<26}");
        if discrete {
            let _ = write!(table, "{:>9}    ", damp_number(p.norm()));
        }
        let _ = write!(
            table,
            "{:>9}      {:>9}        {:>9}",
            damp_number(z),
            damp_number(w),
            damp_number(if eq.re == 0. {
                f64::INFINITY
            } else {
                -1. / eq.re
            })
        );
    }
    table
}
fn equivalent_pole(p: C, ts: f64) -> C {
    if ts == 0. { p } else { p.ln() / ts }
}
/// Four significant digits, as MATLAB prints LTI coefficients, roots and matrix entries.
fn lti_number(x: f64) -> String {
    if x == 0. {
        return "0".into();
    }
    if !x.is_finite() {
        return format!("{x}");
    }
    let exponent = x.abs().log10().floor() as i32;
    if (-4..4).contains(&exponent) {
        let decimals = (3 - exponent).max(0) as usize;
        let s = format!("{x:.decimals$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').into()
        } else {
            s
        }
    } else {
        format!("{x:.3e}")
    }
}
fn var_power(var: &str, power: usize) -> String {
    match (var, power) {
        (_, 0) => String::new(),
        ("z^-1" | "q^-1", 1) => var.into(),
        ("z^-1" | "q^-1", k) => format!("{}^-{k}", &var[..1]),
        (_, 1) => var.into(),
        (_, k) => format!("{var}^{k}"),
    }
}
fn polynomial(c: &[f64], var: &str) -> String {
    let degree = c.len() - 1;
    let mut out = String::new();
    for (i, &x) in c.iter().enumerate() {
        if x == 0. {
            continue;
        }
        let power = if inverse_variable(var) { i } else { degree - i };
        let mag = lti_number(x.abs());
        if mag == "0" {
            continue;
        }
        let vp = var_power(var, power);
        let term = if vp.is_empty() {
            mag
        } else if mag == "1" {
            vp
        } else {
            format!("{mag} {vp}")
        };
        if out.is_empty() {
            if x < 0. {
                out.push('-');
            }
            out.push_str(&term);
        } else if x < 0. {
            out.push_str(" - ");
            out.push_str(&term);
        } else {
            out.push_str(" + ");
            out.push_str(&term);
        }
    }
    if out.is_empty() { "0".into() } else { out }
}
fn format_factor_power(base: String, count: usize) -> String {
    if count > 1 {
        format!("{base}^{count}")
    } else {
        base
    }
}
fn format_quadratic(c1: f64, c0: f64, var: &str) -> String {
    let mut s = format!("({}", var_power(var, 2));
    if c1 != 0. {
        let sign = if c1 < 0. { '-' } else { '+' };
        let mag = lti_number(c1.abs());
        let v1 = var_power(var, 1);
        if mag == "1" {
            s.push_str(&format!(" {sign} {v1}"));
        } else {
            s.push_str(&format!(" {sign} {mag}{v1}"));
        }
    }
    if c0 != 0. {
        let sign = if c0 < 0. { '-' } else { '+' };
        let mag = lti_number(c0.abs());
        s.push_str(&format!(" {sign} {mag}"));
    }
    s.push(')');
    s
}
fn factored(roots: &[C], var: &str, inverse_power: usize) -> String {
    let mut origin_count = 0usize;
    let mut reals: Vec<(f64, usize)> = Vec::new();
    let mut complex_pool: Vec<C> = Vec::new();
    for &r in roots {
        if r == C::new(0., 0.) {
            origin_count += 1;
        } else if r.im == 0. {
            if let Some((_, count)) = reals.iter_mut().find(|(v, _)| *v == r.re) {
                *count += 1;
            } else {
                reals.push((r.re, 1));
            }
        } else {
            complex_pool.push(r);
        }
    }
    let mut quads: Vec<(f64, f64, usize)> = Vec::new();
    while !complex_pool.is_empty() {
        let r = complex_pool.remove(0);
        let partner_idx = complex_pool
            .iter()
            .enumerate()
            .filter(|(_, p)| (r.im > 0. && p.im < 0.) || (r.im < 0. && p.im > 0.))
            .min_by(|(_, a), (_, b)| (**a - r.conj()).norm().total_cmp(&(**b - r.conj()).norm()))
            .map(|(i, _)| i);
        let (c1, c0) = if let Some(idx) = partner_idx {
            let p = complex_pool.remove(idx);
            (-(r.re + p.re), (r * p).re)
        } else {
            (-2. * r.re, r.re * r.re + r.im * r.im)
        };
        if let Some((_, _, count)) = quads.iter_mut().find(|(a1, a0, _)| *a1 == c1 && *a0 == c0) {
            *count += 1;
        } else {
            quads.push((c1, c0, 1));
        }
    }
    let mut factors = Vec::new();
    // (z-r) = z (1-r z^-1). The net degree supplies the residual
    // power of z^-1; a root at zero contributes no reciprocal factor.
    if inverse_power > 0 {
        factors.push(var_power(var, inverse_power));
    } else if origin_count > 0 && !inverse_variable(var) {
        factors.push(var_power(var, origin_count));
    }
    for (r, count) in reals {
        let mag = lti_number(r.abs());
        let base = if inverse_variable(var) {
            format!("({})", polynomial(&[1., -r], var).replace(' ', ""))
        } else if r < 0. {
            format!("({var}+{mag})")
        } else {
            format!("({var}-{mag})")
        };
        factors.push(format_factor_power(base, count));
    }
    for (c1, c0, count) in quads {
        let base = if inverse_variable(var) {
            format!("({})", polynomial(&[1., c1, c0], var).replace(' ', ""))
        } else {
            format_quadratic(c1, c0, var)
        };
        factors.push(format_factor_power(base, count));
    }
    if factors.is_empty() {
        "1".into()
    } else {
        factors.join(" ")
    }
}
fn zpk_numerator(z: &[C], k: f64, var: &str, inverse_power: usize) -> String {
    let k_str = lti_number(k);
    if k == 0. || k_str == "0" {
        return "0".into();
    }
    let f = factored(z, var, inverse_power);
    if f == "1" {
        return k_str;
    }
    if k_str == "1" {
        f
    } else if k_str == "-1" {
        format!("-{f}")
    } else {
        format!("{k_str} {f}")
    }
}
fn fraction_display(num: &str, den: &str, omit_unit_den: bool) -> String {
    if omit_unit_den && den == "1" {
        return format!("  {num}");
    }
    let width = num.len().max(den.len());
    let num_pad = (width - num.len()) / 2;
    let den_pad = (width - den.len()) / 2;
    format!(
        "{}{num}\n  {}\n{}{den}",
        " ".repeat(2 + num_pad),
        "-".repeat(width),
        " ".repeat(2 + den_pad),
    )
}
fn port_labels(a: &Array, n: usize, fallback: impl Fn(usize) -> String) -> Vec<String> {
    let custom: Vec<String> = match a {
        Array::Character(c) if n == 1 => vec![String::from_utf16_lossy(c.units())],
        Array::Cell(c) => (0..c.len())
            .map(|i| match c.get_linear(i).ok().map(|a| a.as_ref()) {
                Some(Array::Character(ch)) => String::from_utf16_lossy(ch.units()),
                _ => String::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    (0..n)
        .map(|i| match custom.get(i) {
            Some(s) if !s.is_empty() => s.clone(),
            _ => fallback(i + 1),
        })
        .collect()
}
fn ss_block(label: &str, m: &Matrix, row_labels: &[String], col_labels: &[String]) -> String {
    let row_w = row_labels.iter().map(|s| s.len()).max().unwrap_or(0);
    let entries: Vec<Vec<String>> = (0..m.rows())
        .map(|r| {
            (0..m.cols())
                .map(|c| lti_number(m.data()[c * m.rows() + r]))
                .collect()
        })
        .collect();
    let value_w = entries.iter().flatten().map(String::len).max().unwrap_or(0);
    let widths: Vec<_> = col_labels
        .iter()
        .map(|name| name.len().max(value_w) + 2)
        .collect();
    let mut lines = Vec::with_capacity(m.rows() + 2);
    lines.push(format!("  {label} = "));
    let mut hdr = " ".repeat(3 + row_w);
    for (lbl, &width) in col_labels.iter().zip(&widths) {
        hdr.push_str(&format!("{lbl:>width$}"));
    }
    lines.push(hdr);
    for (label, row) in row_labels.iter().zip(entries) {
        let mut line = format!("   {label:<row_w$}");
        for (entry, &width) in row.iter().zip(&widths) {
            line.push_str(&format!("{entry:>width$}"));
        }
        lines.push(line);
    }
    lines.join("\n")
}
fn ss_display(s: &Lti, a: &Matrix, b: &Matrix, c: &Matrix, d: &Matrix) -> String {
    let n = a.rows();
    let x_labels = s.props.state.as_ref().map_or_else(
        || (1..=n).map(|i| format!("x{i}")).collect(),
        |names| port_labels(names, n, |i| format!("x{i}")),
    );
    let u_labels = port_labels(&s.input, b.cols(), |i| format!("u{i}"));
    let y_labels = port_labels(&s.output, c.rows(), |i| format!("y{i}"));
    let mut blocks = Vec::new();
    if n > 0 {
        blocks.push(ss_block("A", a, &x_labels, &x_labels));
        blocks.push(ss_block("B", b, &x_labels, &u_labels));
        blocks.push(ss_block("C", c, &y_labels, &x_labels));
    }
    blocks.push(ss_block("D", d, &y_labels, &u_labels));
    blocks.join("\n \n")
}
fn channel_display(model: &Model, variable: &str) -> Result<String> {
    let (num, den) = match model {
        Model::Tf(n, d) => {
            let (n, d) = public_tf(n, d, variable)?;
            (
                polynomial(n.data(), variable),
                polynomial(d.data(), variable),
            )
        }
        Model::Zpk(z, p, k) => {
            let (np, dp) = if inverse_variable(variable) {
                (
                    p.len().saturating_sub(z.len()),
                    z.len().saturating_sub(p.len()),
                )
            } else {
                (0, 0)
            };
            (
                zpk_numerator(z, *k, variable, np),
                factored(p, variable, dp),
            )
        }
        _ => unreachable!("rational channel required"),
    };
    Ok(fraction_display(&num, &den, den == "1"))
}
pub(super) fn display(e: &Engine, a: &Array) -> Result<Option<String>> {
    if let Some((p, _, _)) = pid_decode(a)? {
        return Ok(Some(pid_display(&p)));
    }
    let Some(s) = decode(e, a)? else {
        return Ok(None);
    };
    let mut body = match &s.model {
        Model::Ss(a, b, c, d) => ss_display(&s, a, b, c, d),
        Model::TfArray(..) | Model::ZpkArray(..) => mimo::display(e, &s)?,
        model => channel_display(model, &s.variable)?,
    };
    body.push_str(if s.ts != 0. || matches!(s.model, Model::Ss(..)) {
        "\n \n"
    } else {
        "\n\n"
    });
    for (label, delay) in ["Input delay", "Output delay", "I/O delay"]
        .into_iter()
        .zip(s.props.delay)
    {
        if delay != 0. {
            body.push_str(&format!("{label}: {}\n", lti_number(delay)));
        }
    }
    if s.has_internal_delays() {
        let internal = s
            .internal_paths()
            .iter()
            .filter_map(|path| {
                let delay = path.delay();
                (delay != 0.).then(|| lti_number(delay))
            })
            .collect::<Vec<_>>()
            .join("  ");
        body.push_str("(values computed with all internal delays set to zero)\n");
        body.push_str(&format!(
            "Internal delays ({}): {internal}\n",
            if s.ts == 0. {
                "seconds"
            } else {
                "sampling periods"
            }
        ));
    }
    if s.ts != 0. && s.delay() != 0. {
        body.push_str(&format!("Delay factor: z^-{}\n", s.delay()));
    }
    if s.ts == -1. {
        body.push_str("Sample time: unspecified\n");
    } else if s.ts > 0. {
        body.push_str(&format!("Sample time: {} seconds\n", lti_number(s.ts)));
    }
    body.push_str(&format!(
        "{}-time {}.",
        if s.ts == 0. { "Continuous" } else { "Discrete" },
        match s.kind() {
            "tf" => "transfer function",
            "zpk" => "zero/pole/gain model",
            _ => "state-space model",
        }
    ));
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_names_and_four_sig_digits() {
        let num = Matrix::from_column_slice(1, 2, &[1., 3.]).unwrap();
        let den = Matrix::from_column_slice(1, 3, &[1., -2., 5.]).unwrap();
        assert_eq!(polynomial(num.data(), "p"), "p + 3");
        assert_eq!(polynomial(den.data(), "q"), "q^2 - 2 q + 5");
        assert_eq!(polynomial(den.data(), "z^-1"), "1 - 2 z^-1 + 5 z^-2");

        let roots = vec![C::new(-1., 2.), C::new(-1., -2.)];
        assert_eq!(factored(&roots, "p", 0), "(p^2 + 2p + 5)");
        assert_eq!(factored(&[C::new(-1., 0.)], "z^-1", 0), "(1+z^-1)");

        assert_eq!(lti_number(-0.0), "0");
        assert_eq!(lti_number(0.9048374), "0.9048");
        assert_eq!(lti_number(0.09516258), "0.09516");
    }
}
