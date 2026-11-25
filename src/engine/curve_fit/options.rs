//! fitoptions: fit settings per fitting method. The options value is a plain
//! struct with the method's fields, so `opts.Lower = [0 0]` and copies behave
//! as values; fit reads it back field by field with validation.
use super::super::{Engine, Value};
use super::model::{FitType, Interp, Kind};
use crate::{Array, CharacterRow, Matrix, Result, StructArray};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Method {
    Nonlinear,
    Linear,
    Interp(Interp),
    None,
}
impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Nonlinear => "NonlinearLeastSquares",
            Method::Linear => "LinearLeastSquares",
            Method::Interp(m) => m.method(),
            Method::None => "None",
        }
    }
    fn parse(text: &str) -> Result<Self> {
        Ok(match text.to_ascii_lowercase().as_str() {
            "nonlinearleastsquares" => Method::Nonlinear,
            "linearleastsquares" => Method::Linear,
            "linearinterpolant" => Method::Interp(Interp::Linear),
            "nearestinterpolant" => Method::Interp(Interp::Nearest),
            "pchipinterpolant" => Method::Interp(Interp::Pchip),
            "cubicsplineinterpolant" => Method::Interp(Interp::Cubic),
            "smoothingspline" => Method::Interp(Interp::Smooth),
            "none" => Method::None,
            "lowessfit" | "biharmonicinterpolant" | "thinplateinterpolant" => {
                return Err(format!(
                    "fitoptions: the '{text}' method is not supported yet"
                ));
            }
            _ => return Err(format!("fitoptions: unknown fitting method '{text}'")),
        })
    }
    fn fields(self) -> &'static [&'static str] {
        match self {
            Method::Nonlinear => &NONLINEAR,
            Method::Linear => &LINEAR,
            Method::Interp(Interp::Smooth) => &SMOOTH,
            Method::Interp(_) | Method::None => &COMMON,
        }
    }
}
const COMMON: [&str; 4] = ["Normalize", "Exclude", "Weights", "Method"];
const SMOOTH: [&str; 5] = [
    "Normalize",
    "Exclude",
    "Weights",
    "Method",
    "SmoothingParam",
];
/// Every option name of every method.
const ALL: [&str; 17] = [
    "Normalize",
    "Exclude",
    "Weights",
    "Method",
    "Robust",
    "StartPoint",
    "Lower",
    "Upper",
    "Algorithm",
    "DiffMinChange",
    "DiffMaxChange",
    "Display",
    "MaxFunEvals",
    "MaxIter",
    "TolFun",
    "TolX",
    "SmoothingParam",
];
const LINEAR: [&str; 7] = [
    "Normalize",
    "Exclude",
    "Weights",
    "Method",
    "Robust",
    "Lower",
    "Upper",
];
const NONLINEAR: [&str; 16] = [
    "Normalize",
    "Exclude",
    "Weights",
    "Method",
    "Robust",
    "StartPoint",
    "Lower",
    "Upper",
    "Algorithm",
    "DiffMinChange",
    "DiffMaxChange",
    "Display",
    "MaxFunEvals",
    "MaxIter",
    "TolFun",
    "TolX",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Robust {
    Off,
    Lar,
    Bisquare,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Display {
    Notify,
    Off,
    Iter,
    Final,
}

#[derive(Clone)]
pub(super) struct Options {
    pub method: Method,
    pub normalize: bool,
    /// Logical mask or indices as given; empty when nothing is excluded.
    pub exclude: Matrix,
    pub weights: Vec<f64>,
    pub robust: Robust,
    pub start: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub levenberg: bool,
    pub diff_min: f64,
    pub diff_max: f64,
    pub display: Display,
    pub max_fev: f64,
    pub max_iter: f64,
    pub tol_fun: f64,
    pub tol_x: f64,
    /// SmoothingParam; None picks the data-dependent default.
    pub smoothing: Option<f64>,
    /// Exclude came from a logical mask (kept for display round trips).
    pub exclude_logical: bool,
}

impl Options {
    pub(super) fn new(method: Method) -> Self {
        Self {
            method,
            normalize: false,
            exclude: Matrix::from_column_vec(0, 0, Vec::new()).unwrap(),
            weights: Vec::new(),
            robust: Robust::Off,
            start: Vec::new(),
            lower: Vec::new(),
            upper: Vec::new(),
            levenberg: false,
            diff_min: 1e-8,
            diff_max: 0.1,
            display: Display::Notify,
            max_fev: 600.,
            max_iter: 400.,
            tol_fun: 1e-6,
            tol_x: 1e-6,
            smoothing: None,
            exclude_logical: false,
        }
    }
    /// Defaults of a model: its method and, for library models, default bounds.
    pub(super) fn for_model(ft: &FitType) -> Self {
        let method = match &ft.kind {
            Kind::Interp(m) => Method::Interp(*m),
            _ if ft.linear() => Method::Linear,
            _ => Method::Nonlinear,
        };
        let mut o = Self::new(method);
        if let Kind::Library(lib) = &ft.kind {
            o.lower = lib.default_lower();
        }
        o
    }
    pub(super) fn is_field(&self, name: &str) -> bool {
        self.method.fields().contains(&name)
    }
    /// The method field matching `given` (case-insensitive, unique prefix).
    fn field(&self, given: &str) -> Result<&'static str> {
        let lower = given.to_ascii_lowercase();
        let all = ALL;
        if let Some(f) = all.iter().find(|f| f.to_ascii_lowercase() == lower) {
            return self.own(f, given);
        }
        let mut matches: Vec<&&str> = all
            .iter()
            .filter(|f| f.to_ascii_lowercase().starts_with(&lower))
            .collect();
        if matches.len() > 1 {
            // A prefix shared with another method's option means this method's one.
            matches.retain(|f| self.method.fields().contains(f));
        }
        match matches.as_slice() {
            [f] if !lower.is_empty() => self.own(f, given),
            [] | [_] => Err(format!("Invalid fit option '{given}'.")),
            _ => Err(format!("Ambiguous fit option '{given}'.")),
        }
    }
    fn own(&self, f: &'static str, given: &str) -> Result<&'static str> {
        if self.method.fields().contains(&f) {
            Ok(f)
        } else {
            Err(format!(
                "'{given}' is not an option of the {} fitting method.",
                self.method.name()
            ))
        }
    }

    /// Set one option from a name-value pair, validating the value.
    pub(super) fn set(&mut self, e: &Engine, name: &str, value: &Value) -> Result<()> {
        let field = self.field(name)?;
        self.set_field(e, field, value)
    }
    fn set_field(&mut self, e: &Engine, field: &str, value: &Value) -> Result<()> {
        let text = || text_of(value).map(|t| t.to_ascii_lowercase());
        let number = |what: &str, positive: bool| -> Result<f64> {
            let v =
                scalar(value).ok_or_else(|| format!("fitoptions: {what} must be a real scalar"))?;
            if v.is_nan() || (positive && v <= 0.) {
                return Err(format!("fitoptions: {what} must be a positive scalar"));
            }
            Ok(v)
        };
        match field {
            "Normalize" => {
                self.normalize = match (text().as_deref(), scalar(value)) {
                    (Some("on"), _) => true,
                    (Some("off"), _) => false,
                    (None, Some(v)) => v != 0.,
                    _ => return Err("fitoptions: Normalize must be 'on' or 'off'".into()),
                }
            }
            "Exclude" => {
                let (m, logical) = match value {
                    Value::Logical(l) => (l.to_numeric(&e.cancel)?, true),
                    Value::Matrix(m) => (m.clone(), false),
                    _ => {
                        return Err(
                            "fitoptions: Exclude must be a logical vector or a vector of indices"
                                .into(),
                        );
                    }
                };
                if !m.is_empty() && m.rows() != 1 && m.cols() != 1 {
                    return Err("fitoptions: Exclude must be a vector".into());
                }
                self.exclude = m;
                self.exclude_logical = logical;
            }
            "Weights" => {
                let w = vector(e, value).ok_or("fitoptions: Weights must be a real vector")?;
                if w.iter().any(|v| !v.is_finite() || *v < 0.) {
                    return Err("fitoptions: Weights must be finite and nonnegative".into());
                }
                self.weights = w;
            }
            "Method" => {
                let t = text_of(value).ok_or("fitoptions: Method must be text")?;
                let m = Method::parse(&t)?;
                if m != self.method {
                    return Err(format!(
                        "fitoptions: Method is {}; create new options with fitoptions('Method', '{}') to change it",
                        self.method.name(),
                        m.name()
                    ));
                }
            }
            "Robust" => {
                self.robust = match text().as_deref() {
                    Some("off") => Robust::Off,
                    Some("on") | Some("bisquare") => Robust::Bisquare,
                    Some("lar") => Robust::Lar,
                    _ => return Err("fitoptions: Robust must be 'off', 'LAR' or 'Bisquare'".into()),
                }
            }
            "StartPoint" | "Lower" | "Upper" => {
                let v = vector(e, value)
                    .ok_or_else(|| format!("fitoptions: {field} must be a real vector"))?;
                if v.iter().any(|x| x.is_nan()) {
                    return Err(format!("fitoptions: {field} must not contain NaN"));
                }
                if field == "StartPoint" && v.iter().any(|x| x.is_infinite()) {
                    return Err("fitoptions: StartPoint must be finite".into());
                }
                match field {
                    "StartPoint" => self.start = v,
                    "Lower" => self.lower = v,
                    _ => self.upper = v,
                }
            }
            "Algorithm" => {
                self.levenberg = match text().as_deref() {
                    Some("trust-region") => false,
                    Some("levenberg-marquardt") => true,
                    _ => {
                        return Err(
                            "fitoptions: Algorithm must be 'Trust-Region' or 'Levenberg-Marquardt'"
                                .into(),
                        );
                    }
                }
            }
            "Display" => {
                self.display = match text().as_deref() {
                    Some("notify") => Display::Notify,
                    Some("off") => Display::Off,
                    Some("iter") => Display::Iter,
                    Some("final") => Display::Final,
                    _ => {
                        return Err(
                            "fitoptions: Display must be 'notify', 'off', 'iter' or 'final'".into(),
                        );
                    }
                }
            }
            "DiffMinChange" => self.diff_min = number(field, true)?,
            "DiffMaxChange" => self.diff_max = number(field, true)?,
            "MaxFunEvals" | "MaxIter" => {
                let v = number(field, true)?;
                if v.fract() != 0. && v.is_finite() {
                    return Err(format!("fitoptions: {field} must be a positive integer"));
                }
                if field == "MaxIter" {
                    self.max_iter = v;
                } else {
                    self.max_fev = v;
                }
            }
            "TolFun" => self.tol_fun = number(field, true)?,
            "TolX" => self.tol_x = number(field, true)?,
            "SmoothingParam" => {
                self.smoothing = match vector(e, value) {
                    Some(v) if v.is_empty() => None,
                    Some(v) if v.len() == 1 && (0. ..=1.).contains(&v[0]) => Some(v[0]),
                    _ => {
                        return Err(
                            "fitoptions: SmoothingParam must be a scalar between 0 and 1 (or [] for the default)"
                                .into(),
                        );
                    }
                }
            }
            _ => return Err(format!("Invalid fit option '{field}'.")),
        }
        Ok(())
    }

    /// Fields of an options struct (fitoptions output, possibly edited) on top
    /// of these defaults. Fields of other methods are ignored; empty bounds and
    /// start points keep the model defaults.
    pub(super) fn merge_struct(&mut self, e: &Engine, s: &StructArray) -> Result<()> {
        if s.len() != 1 {
            return Err("fit: fit options must be a scalar fitoptions value".into());
        }
        for (k, name) in s.field_names().iter().enumerate() {
            if !ALL.contains(&name.as_str()) {
                return Err(format!("Invalid fit option '{name}'."));
            }
            if name == "Method" || !self.is_field(name) {
                continue;
            }
            let value = Value::from(s.get(k, 0)?.as_ref().clone());
            if matches!(name.as_str(), "StartPoint" | "Lower" | "Upper")
                && vector(e, &value).is_some_and(|v| v.is_empty())
            {
                continue;
            }
            self.set_field(e, name, &value)?;
        }
        Ok(())
    }

    /// The options as a struct with the method's fields in order.
    pub(super) fn to_value(&self, e: &Engine) -> Result<Value> {
        let text = |t: &str| -> Result<Arc<Array>> {
            Ok(Arc::new(Array::Character(CharacterRow::from_text(
                t, &e.cancel,
            )?)))
        };
        let num = |v: f64| Arc::new(Array::Numeric(Matrix::scalar(v)));
        let row = |v: &[f64]| -> Result<Arc<Array>> {
            Ok(Arc::new(Array::Numeric(if v.is_empty() {
                Matrix::from_column_vec(1, 0, Vec::new())?
            } else {
                Matrix::from_column_vec(1, v.len(), v.to_vec())?
            })))
        };
        let mut names = Vec::new();
        let mut slots = Vec::new();
        for f in self.method.fields() {
            let v = match *f {
                "Normalize" => text(if self.normalize { "on" } else { "off" })?,
                "Exclude" => {
                    if self.exclude_logical {
                        let bits: Vec<u8> = self
                            .exclude
                            .data()
                            .iter()
                            .map(|v| u8::from(*v != 0.))
                            .collect();
                        Arc::new(Array::Logical(crate::LogicalMatrix::from_column_vec(
                            self.exclude.rows(),
                            self.exclude.cols(),
                            bits,
                            &e.cancel,
                        )?))
                    } else {
                        Arc::new(Array::Numeric(self.exclude.clone()))
                    }
                }
                "Weights" => Arc::new(Array::Numeric(if self.weights.is_empty() {
                    Matrix::from_column_vec(0, 0, Vec::new())?
                } else {
                    Matrix::from_column_vec(self.weights.len(), 1, self.weights.clone())?
                })),
                "Method" => text(self.method.name())?,
                "Robust" => text(match self.robust {
                    Robust::Off => "Off",
                    Robust::Lar => "LAR",
                    Robust::Bisquare => "Bisquare",
                })?,
                "StartPoint" => row(&self.start)?,
                "Lower" => row(&self.lower)?,
                "Upper" => row(&self.upper)?,
                "Algorithm" => text(if self.levenberg {
                    "Levenberg-Marquardt"
                } else {
                    "Trust-Region"
                })?,
                "DiffMinChange" => num(self.diff_min),
                "DiffMaxChange" => num(self.diff_max),
                "Display" => text(match self.display {
                    Display::Notify => "Notify",
                    Display::Off => "Off",
                    Display::Iter => "Iter",
                    Display::Final => "Final",
                })?,
                "MaxFunEvals" => num(self.max_fev),
                "MaxIter" => num(self.max_iter),
                "TolFun" => num(self.tol_fun),
                "SmoothingParam" => match self.smoothing {
                    Some(p) => num(p),
                    None => Arc::new(Array::Numeric(Matrix::from_column_vec(0, 0, Vec::new())?)),
                },
                _ => num(self.tol_x),
            };
            names.push((*f).to_owned());
            slots.push(v);
        }
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1, 1, names, slots, &e.cancel,
        )?)))
    }
}

pub(super) fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}
fn scalar(v: &Value) -> Option<f64> {
    match v {
        Value::Matrix(m) if m.len() == 1 => Some(m.data()[0]),
        Value::Logical(l) if l.len() == 1 => Some(f64::from(l.data()[0])),
        _ => None,
    }
}
/// Real numeric (or logical) values as a flat vector; None for other types.
pub(super) fn vector(e: &Engine, v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Matrix(m) => Some(m.data().to_vec()),
        Value::Logical(l) => l.to_numeric(&e.cancel).ok().map(|m| m.data().to_vec()),
        Value::Container(Array::Typed(_)) => {
            v.clone().matrix(&e.cancel).ok().map(|m| m.data().to_vec())
        }
        _ => None,
    }
}

/// fitoptions(...) in its documented forms.
pub(super) fn fitoptions(
    e: &mut Engine,
    args: &[Value],
    model: Option<FitType>,
    out: &mut super::super::Execution,
) -> Result<Value> {
    let (mut opts, rest) = match (model, args.first()) {
        (Some(ft), _) => (Options::for_model(&ft), &args[1..]),
        (None, None) => (Options::new(Method::None), args),
        (None, Some(Value::Container(Array::Struct(s)))) => {
            let mut o = Options::new(struct_method(s)?);
            o.merge_struct(e, s)?;
            for (k, name) in s.field_names().iter().enumerate() {
                // Keep explicit empty values and bounds as they are.
                if matches!(name.as_str(), "StartPoint" | "Lower" | "Upper") {
                    let value = Value::from(s.get(k, 0)?.as_ref().clone());
                    o.set_field(e, name, &value)?;
                }
            }
            (o, &args[1..])
        }
        (None, Some(first)) => {
            let text = text_of(first).ok_or(
                "fitoptions: expected a library model name, a fittype, a fitoptions value or 'Method'",
            )?;
            if text.eq_ignore_ascii_case("method") {
                let m = args
                    .get(1)
                    .and_then(text_of)
                    .ok_or("fitoptions: 'Method' needs a method name")?;
                (Options::new(Method::parse(&m)?), &args[2..])
            } else {
                let ft = super::model::from_value(e, first, &Default::default(), out)?;
                (Options::for_model(&ft), &args[1..])
            }
        }
    };
    if rest.len() % 2 != 0 {
        return Err("fitoptions: name-value arguments must come in pairs".into());
    }
    for pair in rest.chunks(2) {
        let name = text_of(&pair[0]).ok_or("fitoptions: option names must be text")?;
        opts.set(e, &name, &pair[1])?;
    }
    opts.to_value(e)
}

fn struct_method(s: &StructArray) -> Result<Method> {
    let Some(k) = s.field_index("Method") else {
        return Err("fitoptions: the options struct has no Method field".into());
    };
    let v = Value::from(s.get(k, 0)?.as_ref().clone());
    Method::parse(&text_of(&v).ok_or("fitoptions: Method must be text")?)
}
