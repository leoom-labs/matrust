//! Curve fitting core: fittype, fit, fitoptions and the cfit / sfit objects
//! (evaluation, coefficients, confint, predint, differentiate, integrate,
//! plot), built from the public function documentation. fittype, cfit and
//! sfit are immutable object states (src/engine/objects.rs ObjState::Fit);
//! fitoptions values are plain structs.
mod calculus;
mod fit;
mod inference;
mod library;
mod model;
mod options;
mod plot;
mod smooth;
mod solve;
mod start;

use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, FunctionHandle, LogicalMatrix, Matrix, Result, StructArray,
    syntax::Expr,
};
use model::{FitType, Knots};
use std::sync::Arc;

/// A fitted model: cfit (one independent variable) or sfit (two).
#[derive(Clone)]
pub(in crate::engine) struct Fitted {
    pub ft: FitType,
    pub coef: Vec<f64>,
    pub problem: Vec<f64>,
    /// Unscaled covariance inv(J'J) (p-by-p, column-major); empty when unknown.
    pub cov: Vec<f64>,
    /// Weighted sse/dfe of the fit; NaN when unknown.
    pub mse: f64,
    /// Error degrees of freedom; NaN for objects made by cfit/sfit.
    pub dfe: f64,
    /// (center, scale) per independent variable when fitted with Normalize on.
    pub norm: Vec<(f64, f64)>,
    pub knots: Option<Knots>,
    /// Data range per independent variable, in data units.
    pub range: Vec<(f64, f64)>,
}

#[derive(Clone)]
pub(in crate::engine) enum FitObj {
    Type(FitType),
    Fit(Fitted),
}

/// Names answered for any arguments.
const NAMES: &[&str] = &[
    "fit",
    "fittype",
    "fitoptions",
    "cfit",
    "sfit",
    "confint",
    "predint",
    "coeffnames",
    "coeffvalues",
    "numcoeffs",
    "numargs",
    "dependnames",
    "indepnames",
    "probnames",
    "probvalues",
    "islinear",
    "category",
    "differentiate",
    "integrate",
];
/// Methods of fittype, cfit and sfit (`methods(f)`).
pub(in crate::engine) const METHODS: &[&str] = &[
    "argnames",
    "category",
    "coeffnames",
    "coeffvalues",
    "confint",
    "dependnames",
    "differentiate",
    "feval",
    "fitoptions",
    "formula",
    "indepnames",
    "integrate",
    "islinear",
    "numargs",
    "numcoeffs",
    "plot",
    "predint",
    "probnames",
    "probvalues",
    "type",
];
impl FitObj {
    pub(in crate::engine) fn class(&self) -> &'static str {
        match self {
            FitObj::Type(_) => "fittype",
            FitObj::Fit(f) if f.ft.surface() => "sfit",
            FitObj::Fit(_) => "cfit",
        }
    }
    pub(in crate::engine) fn ft(&self) -> &FitType {
        match self {
            FitObj::Type(t) => t,
            FitObj::Fit(f) => &f.ft,
        }
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        match self {
            FitObj::Type(t) => t.bytes(),
            FitObj::Fit(f) => {
                f.ft.bytes()
                    + 8 * (f.coef.len() + f.problem.len() + f.cov.len())
                    + f.knots.as_ref().map_or(0, |k| {
                        16 * k.x.len() + k.smooth.as_ref().map_or(0, |s| 40 * s.pp.coefs.len())
                    })
                    + 64
            }
        }
    }
    /// Function handles held (their captured arrays are traced by the collector).
    pub(in crate::engine) fn handles(&self) -> Vec<&FunctionHandle> {
        self.ft().handles()
    }
    pub(in crate::engine) fn body(&self) -> String {
        match self {
            FitObj::Type(t) => format!(
                "     {}:\n     f({}) = {}\n",
                t.heading(),
                t.args().join(","),
                t.formula
            ),
            FitObj::Fit(f) => fitted_body(f),
        }
    }
    pub(in crate::engine) fn property_names(&self) -> Vec<String> {
        match self {
            FitObj::Type(_) => Vec::new(),
            FitObj::Fit(f) => f.ft.coeffs.iter().chain(&f.ft.problems).cloned().collect(),
        }
    }
    /// `f.name` for a coefficient or problem parameter.
    pub(in crate::engine) fn property(&self, name: &str) -> Result<Value> {
        if let FitObj::Fit(f) = self {
            if let Some(k) = f.ft.coeffs.iter().position(|c| c == name) {
                if f.knots.is_some() {
                    return Err(format!(
                        "The coefficient '{name}' of an interpolant is its piecewise data; evaluate the fit instead"
                    ));
                }
                return Ok(Value::Matrix(Matrix::scalar(f.coef[k])));
            }
            if let Some(k) = f.ft.problems.iter().position(|c| c == name) {
                return Ok(Value::Matrix(Matrix::scalar(f.problem[k])));
            }
        }
        Err(format!(
            "No coefficient, problem parameter or method '{name}' for class '{}'.",
            self.class()
        ))
    }
}

/// Four significant digits, as %.4g.
fn g4(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v < 0. { "-Inf" } else { "Inf" }.into();
    }
    if v == 0. {
        return "0".into();
    }
    let sci = format!("{v:.3e}");
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if !(-4..4).contains(&exp) {
        let m = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    let decimals = (3 - exp).max(0) as usize;
    let fixed = format!("{v:.decimals$}");
    if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        fixed
    }
}

fn fitted_body(f: &Fitted) -> String {
    let ft = &f.ft;
    let mut s = format!(
        "     {}:\n     f({}) = {}\n",
        ft.heading(),
        ft.indep.join(","),
        ft.formula
    );
    for (name, (m, sd)) in ft.indep.iter().zip(&f.norm) {
        s.push_str(&format!(
            "       where {name} is normalized by mean {} and std {}\n",
            g4(*m),
            g4(*sd)
        ));
    }
    if let Some(k) = &f.knots {
        match &k.smooth {
            Some(sm) => s.push_str(&smooth::describe(sm)),
            None => s.push_str(&format!("     Interpolates {} data points.\n", k.x.len())),
        }
        return s;
    }
    if !ft.problems.is_empty() {
        s.push_str("     Problem parameters:\n");
        let w = ft.problems.iter().map(String::len).max().unwrap_or(1);
        for (n, v) in ft.problems.iter().zip(&f.problem) {
            s.push_str(&format!("       {n:>w$} = {}\n", g4(*v)));
        }
    }
    let w = ft.coeffs.iter().map(String::len).max().unwrap_or(1);
    if f.dfe.is_nan() {
        s.push_str("     Coefficients:\n");
        for (n, v) in ft.coeffs.iter().zip(&f.coef) {
            s.push_str(&format!("       {n:>w$} = {}\n", g4(*v)));
        }
    } else {
        s.push_str("     Coefficients (with 95% confidence bounds):\n");
        let (lo, hi) = inference::bounds(f, 0.95);
        for (k, n) in ft.coeffs.iter().enumerate() {
            s.push_str(&format!(
                "       {n:>w$} = {}  ({}, {})\n",
                g4(f.coef[k]),
                g4(lo[k]),
                g4(hi[k])
            ));
        }
    }
    s
}

pub(super) fn object(e: &Engine, v: &Value) -> Option<Arc<FitObj>> {
    super::objects::fit_value(e, v)
}
fn new_object(e: &mut Engine, obj: FitObj) -> Result<Value> {
    e.fit_new(obj)
}

fn text(e: &Engine, t: &str) -> Result<Arc<Array>> {
    Ok(Arc::new(Array::Character(CharacterRow::from_text(
        t, &e.cancel,
    )?)))
}
fn make_struct(e: &Engine, fields: Vec<(&str, Value)>) -> Result<Value> {
    let mut names = Vec::new();
    let mut slots = Vec::new();
    for (n, v) in fields {
        names.push(n.to_owned());
        slots.push(Arc::new(v.array(&e.cancel)?));
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, &e.cancel,
    )?)))
}
/// A column cell array of texts (0-by-0 when empty).
fn names_column(e: &Engine, names: &[String]) -> Result<Value> {
    let items = names
        .iter()
        .map(|n| text(e, n))
        .collect::<Result<Vec<_>>>()?;
    let rows = items.len();
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        rows,
        usize::from(rows > 0),
        items,
        &e.cancel,
    )?)))
}
fn row(v: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        1,
        v.len(),
        v.to_vec(),
    )?))
}

/// Query points in data units (one Vec per independent variable), the shape
/// of the result and how many arguments they took: x for curves; x, y or
/// [x y] for surfaces.
/// Evaluation points, output shape, point count.
type QueryPoints = (Vec<Vec<f64>>, (usize, usize), usize);

fn query_points(e: &Engine, ft: &FitType, rest: &[Value], who: &str) -> Result<QueryPoints> {
    let Some(first) = rest.first() else {
        return Err(format!("{who}: expected the points to evaluate at"));
    };
    let x = fit::real_matrix(e, first, who, "x")?;
    if !ft.surface() {
        return Ok((vec![x.data().to_vec()], (x.rows(), x.cols()), 1));
    }
    if let Some(second) = rest.get(1)
        && options::text_of(second).is_none()
    {
        let y = fit::real_matrix(e, second, who, "y")?;
        if x.len() != y.len() {
            return Err(format!(
                "{who}: x and y must have the same number of elements"
            ));
        }
        return Ok((
            vec![x.data().to_vec(), y.data().to_vec()],
            (x.rows(), x.cols()),
            2,
        ));
    }
    if x.cols() != 2 {
        return Err(format!(
            "{who}: give x and y, or one matrix [x y] with two columns"
        ));
    }
    let n = x.rows();
    Ok((
        vec![x.data()[..n].to_vec(), x.data()[n..].to_vec()],
        (n, 1),
        1,
    ))
}

/// Points in the model's coordinates (centered and scaled when normalized).
fn normalize(f: &Fitted, xs: &[Vec<f64>]) -> Vec<Vec<f64>> {
    xs.iter()
        .enumerate()
        .map(|(k, x)| match f.norm.get(k) {
            Some(&(m, s)) => x.iter().map(|v| (v - m) / s).collect(),
            None => x.clone(),
        })
        .collect()
}

fn eval_fitted(
    e: &mut Engine,
    f: &Fitted,
    xs: &[Vec<f64>],
    out: &mut Execution,
) -> Result<Vec<f64>> {
    let xs = normalize(f, xs);
    let refs: Vec<&[f64]> = xs.iter().map(Vec::as_slice).collect();
    if refs[0].is_empty() {
        return Ok(Vec::new());
    }
    model::values(e, &f.ft, &f.coef, &f.problem, f.knots.as_ref(), &refs, out)
}

/// `f(x)`, `f(x, y)`, `feval(f, ...)`; for a fittype `ft(coefficients..., problems..., x)`.
fn evaluate(e: &mut Engine, obj: &FitObj, args: &[Value], out: &mut Execution) -> Result<Value> {
    match obj {
        FitObj::Fit(f) => {
            let (xs, (rows, cols), used) = query_points(e, &f.ft, args, "feval")?;
            if used != args.len() {
                return Err(format!(
                    "Too many input arguments: a {} takes {}",
                    obj.class(),
                    if f.ft.surface() {
                        "x and y (or [x y])"
                    } else {
                        "x"
                    }
                ));
            }
            let v = eval_fitted(e, f, &xs, out)?;
            Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, v)?))
        }
        FitObj::Type(t) => {
            let names = t.args();
            if args.len() != names.len() {
                return Err(format!(
                    "This fittype takes {} inputs ({}).",
                    names.len(),
                    names.join(", ")
                ));
            }
            let nc = t.coeffs.len() + t.problems.len();
            let mut scalars = Vec::with_capacity(nc);
            for (v, n) in args[..nc].iter().zip(&names) {
                let m = fit::real_matrix(e, v, "feval", n)?;
                if m.len() != 1 {
                    return Err(format!(
                        "feval: the coefficient or problem value {n} must be a scalar"
                    ));
                }
                scalars.push(m.data()[0]);
            }
            if t.interp().is_some() {
                return Err("feval: an interpolant fittype has no data; fit it first".into());
            }
            let first = fit::real_matrix(e, &args[nc], "feval", &t.indep[0])?;
            let mut xs = vec![first.data().to_vec()];
            if t.surface() {
                let y = fit::real_matrix(e, &args[nc + 1], "feval", &t.indep[1])?;
                if y.len() != first.len() {
                    return Err("feval: x and y must have the same number of elements".into());
                }
                xs.push(y.data().to_vec());
            }
            let refs: Vec<&[f64]> = xs.iter().map(Vec::as_slice).collect();
            let (coef, problem) = scalars.split_at(t.coeffs.len());
            let v = if refs[0].is_empty() {
                Vec::new()
            } else {
                model::values(e, t, coef, problem, None, &refs, out)?
            };
            Ok(Value::Matrix(Matrix::from_column_vec(
                first.rows(),
                first.cols(),
                v,
            )?))
        }
    }
}

fn spec_from(args: &[Value]) -> Result<model::Spec> {
    if !args.len().is_multiple_of(2) {
        return Err("fittype: name-value arguments must come in pairs".into());
    }
    let mut spec = model::Spec::default();
    for pair in args.chunks(2) {
        let name = options::text_of(&pair[0]).ok_or("fittype: option names must be text")?;
        match name.to_ascii_lowercase().as_str() {
            "independent" => spec.independent = Some(model::name_list(&pair[1], "independent")?),
            "dependent" => {
                let v = model::name_list(&pair[1], "dependent")?;
                if v.len() != 1 {
                    return Err("fittype: give one dependent variable".into());
                }
                spec.dependent = Some(v[0].clone());
            }
            "coefficients" => spec.coefficients = Some(model::name_list(&pair[1], "coefficients")?),
            "problem" => spec.problem = Some(model::name_list(&pair[1], "problem")?),
            "options" => {
                return Err("fittype: the 'options' argument is not supported yet; pass fit options to fit instead".into());
            }
            _ => return Err(format!("fittype: unknown option '{name}'")),
        }
    }
    Ok(spec)
}

/// cfit(fittype, coefficients..., problems...) and sfit(...).
fn construct(e: &mut Engine, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
    let Some(first) = args.first() else {
        return Err(format!(
            "{name}: expected {name}(fittype, coefficient values...)"
        ));
    };
    let ft = match object(e, first) {
        Some(obj) => obj.ft().clone(),
        None => model::from_value(e, first, &Default::default(), out)?,
    };
    if ft.interp().is_some() {
        return Err(format!("{name}: interpolants are made by fit"));
    }
    if (name == "sfit") != ft.surface() {
        return Err(format!(
            "{name}: this fittype has {} independent variable(s); use {}",
            ft.indep.len(),
            if ft.surface() { "sfit" } else { "cfit" }
        ));
    }
    let need = ft.coeffs.len() + ft.problems.len();
    if args.len() - 1 != need {
        return Err(format!(
            "{name}: give {need} values ({})",
            ft.coeffs
                .iter()
                .chain(&ft.problems)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut vals = Vec::with_capacity(need);
    for v in &args[1..] {
        let m = fit::real_matrix(e, v, name, "a coefficient value")?;
        if m.len() != 1 {
            return Err(format!(
                "{name}: coefficient and problem values must be scalars"
            ));
        }
        vals.push(m.data()[0]);
    }
    let problem = vals.split_off(ft.coeffs.len());
    let k = ft.indep.len();
    new_object(
        e,
        FitObj::Fit(Fitted {
            ft,
            coef: vals,
            problem,
            cov: Vec::new(),
            mse: f64::NAN,
            dfe: f64::NAN,
            norm: Vec::new(),
            knots: None,
            range: vec![(f64::NAN, f64::NAN); k],
        }),
    )
}

/// Names with several outputs: [f, gof, output] = fit(...), [fx, fxx] =
/// differentiate(...), [ci, y] = predint(...).
pub(in crate::engine) fn multi_output(name: &str) -> bool {
    matches!(name, "fit" | "differentiate" | "predint")
}

fn class_of(e: &Engine, v: Option<&Value>) -> String {
    match v {
        None => "double".into(),
        Some(v) => match object(e, v) {
            Some(o) => o.class().into(),
            None => match v {
                Value::Matrix(_) | Value::Complex(_) | Value::Void => "double".into(),
                Value::Logical(_) => "logical".into(),
                Value::Text(_) => "char".into(),
                Value::QuotedString(_) => "string".into(),
                Value::Container(a) => a.class().into(),
            },
        },
    }
}

/// Builtin calls: the constructors, fit, fitoptions, and every call whose
/// first argument is a fit object. None leaves the call to other builtins.
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    let obj = if e.obj_live() {
        args.first().and_then(|v| object(e, v))
    } else {
        None
    };
    if obj.is_none() && !NAMES.contains(&name) {
        return Ok(None);
    }
    match name {
        "fit" => return fit::fit(e, args, nargout.max(1), out).map(Some),
        "fittype" => {
            let Some(first) = args.first() else {
                return Err(
                    "fittype: expected fittype(libraryModelName) or fittype(expression, ...)"
                        .into(),
                );
            };
            if let Some(o) = &obj {
                if args.len() > 1 {
                    return Err("fittype: too many input arguments".into());
                }
                return Ok(Some(vec![new_object(e, FitObj::Type(o.ft().clone()))?]));
            }
            let spec = spec_from(&args[1..])?;
            let ft = model::from_value(e, first, &spec, out)?;
            return Ok(Some(vec![new_object(e, FitObj::Type(ft))?]));
        }
        "fitoptions" => {
            let model = obj.as_ref().map(|o| o.ft().clone());
            return options::fitoptions(e, args, model, out).map(|v| Some(vec![v]));
        }
        "cfit" | "sfit" => return construct(e, name, args, out).map(|v| Some(vec![v])),
        _ => {}
    }
    let Some(obj) = obj else {
        return Err(e.raise(
            format!(
                "Undefined function '{name}' for input arguments of type '{}'.",
                class_of(e, args.first())
            ),
            "MATLAB:UndefinedFunction".into(),
        ));
    };
    method(e, &obj, name, &args[1..], nargout, out)
}

fn fitted<'a>(obj: &'a FitObj, name: &str) -> Result<&'a Fitted> {
    match obj {
        FitObj::Fit(f) => Ok(f),
        FitObj::Type(_) => Err(format!(
            "{name}: needs a fit result (cfit or sfit); fit the fittype to data first"
        )),
    }
}

/// `name(obj, rest...)` for the fit methods. None for names the generic
/// object code answers (class, size, disp, ...).
fn method(
    e: &mut Engine,
    obj: &FitObj,
    name: &str,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Option<Vec<Value>>> {
    let ft = obj.ft();
    let no_args = |rest: &[Value]| -> Result<()> {
        if rest.is_empty() {
            Ok(())
        } else {
            Err(format!("{name}: too many input arguments"))
        }
    };
    let one = |v: Value| Ok(Some(vec![v]));
    match name {
        "feval" => one(evaluate(e, obj, rest, out)?),
        "coeffnames" => {
            no_args(rest)?;
            one(names_column(e, &ft.coeffs)?)
        }
        "coeffvalues" => {
            no_args(rest)?;
            let f = fitted(obj, name)?;
            if f.knots.is_some() {
                return Err(
                    "coeffvalues: an interpolant's coefficient is its piecewise data".into(),
                );
            }
            one(row(&f.coef)?)
        }
        "probnames" => {
            no_args(rest)?;
            one(names_column(e, &ft.problems)?)
        }
        "probvalues" => {
            no_args(rest)?;
            one(row(&fitted(obj, name)?.problem)?)
        }
        "numcoeffs" | "numargs" => {
            no_args(rest)?;
            let n = if name == "numcoeffs" {
                ft.coeffs.len()
            } else {
                ft.args().len()
            };
            one(Value::Matrix(Matrix::scalar(n as f64)))
        }
        "argnames" => {
            no_args(rest)?;
            one(names_column(e, &ft.args())?)
        }
        "dependnames" => {
            no_args(rest)?;
            one(names_column(e, std::slice::from_ref(&ft.dep))?)
        }
        "indepnames" => {
            no_args(rest)?;
            one(names_column(e, &ft.indep)?)
        }
        "formula" => {
            no_args(rest)?;
            one(Value::Text(ft.formula.clone()))
        }
        "type" => {
            no_args(rest)?;
            one(Value::Text(ft.type_name()))
        }
        "category" => {
            no_args(rest)?;
            one(Value::Text(ft.category().into()))
        }
        "islinear" => {
            no_args(rest)?;
            one(Value::Logical(LogicalMatrix::scalar(ft.linear())))
        }
        "confint" => one(inference::confint(e, fitted(obj, name)?, rest)?),
        "predint" => inference::predint(e, fitted(obj, name)?, rest, nargout, out).map(Some),
        "differentiate" => {
            calculus::differentiate(e, fitted(obj, name)?, rest, nargout.max(1), out).map(Some)
        }
        "integrate" => one(calculus::integrate(e, fitted(obj, name)?, rest, out)?),
        "plot" => plot::plot(e, fitted(obj, name)?, rest, nargout, out).map(Some),
        "isa" => {
            let [kind] = rest else {
                return Err("isa: expected one class name".into());
            };
            let kind = options::text_of(kind).unwrap_or_default();
            let yes = kind == obj.class() || kind == "fittype";
            one(Value::Logical(LogicalMatrix::scalar(yes)))
        }
        "properties" | "fieldnames" => one(names_column(e, &obj.property_names())?),
        "methods" => {
            let mut names: Vec<String> = METHODS.iter().map(|s| (*s).to_owned()).collect();
            names.sort();
            one(names_column(e, &names)?)
        }
        "class" | "isobject" | "isstruct" | "iscell" | "isnumeric" | "ischar" | "islogical"
        | "isfloat" | "isequal" | "disp" | "display" | "length" | "size" | "isempty" | "numel"
        | "ndims" | "isvalid" | "eq" | "ne" => Ok(None),
        _ => Err(e.raise(
            format!(
                "Undefined function '{name}' for input arguments of type '{}'.",
                obj.class()
            ),
            "MATLAB:UndefinedFunction".into(),
        )),
    }
}

/// From objects.rs: `name(obj, ...)` with one result (statements and
/// `obj.method(...)`); None for the generic object queries.
pub(in crate::engine) fn object_method(
    e: &mut Engine,
    obj: &FitObj,
    name: &str,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    if name == "methods" && rest.is_empty() {
        // Statement form: list the methods.
        let mut names: Vec<&str> = METHODS.to_vec();
        names.sort();
        let mut listing = format!("Methods for class {}:\n", obj.class());
        if !e.display.compact {
            listing.insert(0, '\n');
            listing.push('\n');
        }
        for n in names {
            listing.push_str(&format!("    {n}\n"));
        }
        if e.display.compact {
            listing.pop();
        }
        e.emit(out, listing)?;
        return Ok(Some(Value::Void));
    }
    let nargout = usize::from(name != "plot");
    Ok(method(e, obj, name, rest, nargout, out)?.map(|mut v| {
        if v.is_empty() {
            Value::Void
        } else {
            v.swap_remove(0)
        }
    }))
}

/// `obj(args)`.
pub(in crate::engine) fn subsref(
    e: &mut Engine,
    obj: &FitObj,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    evaluate(e, obj, args, out)
}

/// `obj.name` and `obj.name(args)`: coefficients and problem parameters
/// first, then methods.
pub(in crate::engine) fn member(
    e: &mut Engine,
    obj: &FitObj,
    name: &str,
    args: Option<&[Expr]>,
    out: &mut Execution,
) -> Result<Value> {
    if obj.property_names().iter().any(|n| n == name) {
        let value = obj.property(name)?;
        return match args {
            Some(args) if !args.is_empty() => e.index_value(value, args, out),
            _ => Ok(value),
        };
    }
    if METHODS.contains(&name) && name != "fitoptions" {
        let rest = match args {
            Some(args) => e.function_arguments(args, None, out)?,
            None => Vec::new(),
        };
        return Ok(object_method(e, obj, name, &rest, out)?.unwrap_or(Value::Void));
    }
    obj.property(name)
}

/// `feval(obj, ...)` reached through the statement path.
pub(in crate::engine) fn feval(
    e: &mut Engine,
    args: &[Value],
    out: &mut Execution,
) -> Result<Option<Value>> {
    let Some(obj) = args.first().and_then(|v| object(e, v)) else {
        return Ok(None);
    };
    evaluate(e, &obj, &args[1..], out).map(Some)
}

#[cfg(test)]
mod tests {
    use super::g4;

    #[test]
    fn four_significant_digits_like_percent_g() {
        assert_eq!(g4(2.0134), "2.013");
        assert_eq!(g4(0.49851), "0.4985");
        assert_eq!(g4(1234.6), "1235");
        assert_eq!(g4(12346.), "1.235e+04");
        assert_eq!(g4(0.000012345), "1.234e-05");
        assert_eq!(g4(-3.), "-3");
        assert_eq!(g4(f64::NAN), "NaN");
    }
}
