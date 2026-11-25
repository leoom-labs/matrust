//! fittype values: library, custom nonlinear, custom linear and interpolant
//! models; their construction from text, cell arrays and anonymous functions,
//! and their evaluation (values and derivatives with respect to coefficients).
use super::super::{Engine, Execution, Value};
use super::library::{self, Lib};
use crate::{
    Array, FunctionHandle, Matrix, Result,
    syntax::{Expr, parse_expression},
};
use nalgebra::DMatrix;

#[derive(Clone)]
pub(in crate::engine) enum Kind {
    Library(Lib),
    Custom(Custom),
    Linear(Linear),
    Interp(Interp),
}
#[derive(Clone)]
pub(in crate::engine) struct Custom {
    pub handle: FunctionHandle,
    /// For each handle parameter, its position in [coefficients, problems, independents].
    pub order: Vec<usize>,
}
#[derive(Clone)]
pub(in crate::engine) struct Linear {
    /// One handle per term, taking (problems..., independents...).
    pub handles: Vec<FunctionHandle>,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::engine) enum Interp {
    Linear,
    Nearest,
    Pchip,
    Cubic,
    /// Cubic smoothing spline (fit 'smoothingspline'; csaps).
    Smooth,
}
impl Interp {
    pub(super) fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "linearinterp" => Self::Linear,
            "nearestinterp" => Self::Nearest,
            "pchipinterp" => Self::Pchip,
            "cubicinterp" => Self::Cubic,
            "smoothingspline" => Self::Smooth,
            _ => return None,
        })
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Linear => "linearinterp",
            Self::Nearest => "nearestinterp",
            Self::Pchip => "pchipinterp",
            Self::Cubic => "cubicinterp",
            Self::Smooth => "smoothingspline",
        }
    }
    /// The interp1 method that evaluates it.
    pub(super) fn interp1(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Nearest => "nearest",
            Self::Pchip => "pchip",
            Self::Cubic | Self::Smooth => "spline",
        }
    }
    pub(super) fn method(self) -> &'static str {
        match self {
            Self::Linear => "LinearInterpolant",
            Self::Nearest => "NearestInterpolant",
            Self::Pchip => "PchipInterpolant",
            Self::Cubic => "CubicSplineInterpolant",
            Self::Smooth => "SmoothingSpline",
        }
    }
    fn describe(self) -> &'static str {
        match self {
            Self::Linear => "piecewise linear polynomial computed from p",
            Self::Nearest => "piecewise constant (nearest neighbour) polynomial computed from p",
            Self::Pchip => "shape-preserving piecewise cubic (PCHIP) polynomial computed from p",
            Self::Cubic => "cubic spline computed from p",
            Self::Smooth => "piecewise polynomial computed from p",
        }
    }
}

#[derive(Clone)]
pub(in crate::engine) struct FitType {
    pub kind: Kind,
    pub coeffs: Vec<String>,
    pub problems: Vec<String>,
    pub indep: Vec<String>,
    pub dep: String,
    pub formula: String,
}

/// Interpolant data: sorted distinct sites and their (averaged) values.
#[derive(Clone)]
pub(in crate::engine) struct Knots {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    /// The fitted pieces of a smoothing spline (None for interpolants).
    pub smooth: Option<Box<super::smooth::Smoothed>>,
}

const CONSTANTS: &[&str] = &[
    "pi", "eps", "Inf", "inf", "NaN", "nan", "i", "j", "realmax", "realmin", "true", "false",
];

impl FitType {
    pub(super) fn library(lib: Lib) -> Self {
        let (indep, dep) = if lib.surface() {
            (vec!["x".to_owned(), "y".to_owned()], "z".to_owned())
        } else {
            (vec!["x".to_owned()], "y".to_owned())
        };
        Self {
            kind: Kind::Library(lib),
            coeffs: lib.coeffs(),
            problems: Vec::new(),
            indep,
            dep,
            formula: lib.formula(),
        }
    }
    pub(super) fn interpolant(method: Interp) -> Self {
        Self {
            kind: Kind::Interp(method),
            coeffs: vec!["p".into()],
            problems: Vec::new(),
            indep: vec!["x".into()],
            dep: "y".into(),
            formula: method.describe().into(),
        }
    }
    /// type(): the library name, customnonlinear, customlinear or the interpolant.
    pub(super) fn type_name(&self) -> String {
        match &self.kind {
            Kind::Library(lib) => lib.name(),
            Kind::Custom(_) => "customnonlinear".into(),
            Kind::Linear(_) => "customlinear".into(),
            Kind::Interp(m) => m.name().into(),
        }
    }
    pub(super) fn category(&self) -> &'static str {
        match self.kind {
            Kind::Library(_) => "library",
            Kind::Custom(_) | Kind::Linear(_) => "custom",
            // Public category page: smoothing splines are 'spline'.
            Kind::Interp(Interp::Smooth) => "spline",
            Kind::Interp(_) => "interpolant",
        }
    }
    pub(super) fn linear(&self) -> bool {
        match &self.kind {
            Kind::Library(lib) => lib.linear(),
            Kind::Linear(_) => true,
            Kind::Custom(_) | Kind::Interp(_) => false,
        }
    }
    pub(super) fn interp(&self) -> Option<Interp> {
        match self.kind {
            Kind::Interp(m) => Some(m),
            _ => None,
        }
    }
    pub(super) fn surface(&self) -> bool {
        self.indep.len() == 2
    }
    /// Coefficients, problem parameters and independent variables, in call order.
    pub(super) fn args(&self) -> Vec<String> {
        self.coeffs
            .iter()
            .chain(&self.problems)
            .chain(&self.indep)
            .cloned()
            .collect()
    }
    pub(super) fn handles(&self) -> Vec<&FunctionHandle> {
        match &self.kind {
            Kind::Custom(c) => vec![&c.handle],
            Kind::Linear(l) => l.handles.iter().collect(),
            _ => Vec::new(),
        }
    }
    pub(super) fn bytes(&self) -> usize {
        64 + self.formula.len()
            + self.args().iter().map(String::len).sum::<usize>()
            + self.handles().iter().map(|h| h.bytes()).sum::<usize>()
    }
    /// Short model description for display.
    pub(super) fn heading(&self) -> String {
        match &self.kind {
            Kind::Library(lib) => format!(
                "Library model {} ({})",
                lib.name(),
                if lib.linear() { "linear" } else { "nonlinear" }
            ),
            Kind::Custom(_) => "Custom nonlinear model".into(),
            Kind::Linear(_) => "Custom linear model".into(),
            Kind::Interp(Interp::Smooth) => "Smoothing spline".into(),
            Kind::Interp(m) => format!("Interpolant {}", m.name()),
        }
    }
}

/// fittype name-value options.
#[derive(Default)]
pub(super) struct Spec {
    pub independent: Option<Vec<String>>,
    pub dependent: Option<String>,
    pub coefficients: Option<Vec<String>>,
    pub problem: Option<Vec<String>>,
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        _ => None,
    }
}

/// Text or a cell array of texts as a list of names.
pub(super) fn name_list(v: &Value, what: &str) -> Result<Vec<String>> {
    if let Some(t) = text_of(v) {
        return Ok(vec![t.trim().to_owned()]);
    }
    if let Value::Container(Array::Cell(c)) = v {
        return (0..c.len())
            .map(|i| {
                let item = c.get_linear(i)?;
                text_of(&Value::from(item.as_ref().clone()))
                    .map(|t| t.trim().to_owned())
                    .ok_or_else(|| {
                        format!("fittype: {what} must be a name or a cell array of names")
                    })
            })
            .collect();
    }
    Err(format!(
        "fittype: {what} must be a name or a cell array of names"
    ))
}

fn check_names(names: &[String]) -> Result<()> {
    for (k, n) in names.iter().enumerate() {
        if !crate::struct_array::valid_field_name(n) {
            return Err(format!("fittype: '{n}' is not a valid variable name"));
        }
        if names[..k].contains(n) {
            return Err(format!(
                "fittype: '{n}' is used more than once as a coefficient, problem parameter, independent or dependent variable"
            ));
        }
    }
    Ok(())
}

/// fittype(model, Name, Value, ...) for text, a cell array of terms or an
/// anonymous function.
pub(super) fn from_value(
    e: &mut Engine,
    model: &Value,
    spec: &Spec,
    out: &mut Execution,
) -> Result<FitType> {
    if let Some(text) = text_of(model) {
        let text = text.trim().to_owned();
        if let Some(lib) = library::parse(&text)? {
            no_spec(spec, &text)?;
            return Ok(FitType::library(lib));
        }
        if let Some(m) = Interp::parse(&text) {
            no_spec(spec, &text)?;
            return Ok(FitType::interpolant(m));
        }
        if matches!(
            text.to_ascii_lowercase().as_str(),
            "lowess" | "loess" | "biharmonicinterp" | "thinplateinterp"
        ) {
            return Err(format!(
                "fittype: the '{text}' model is not supported yet; use a library model, a custom equation or linearinterp, nearestinterp, pchipinterp, cubicinterp or smoothingspline"
            ));
        }
        return custom(e, &text, spec, out);
    }
    match model {
        Value::Container(Array::Cell(_)) => linear(e, &name_list(model, "linear model terms")?, spec),
        Value::Container(Array::Handle(h)) => from_handle(e, h, spec),
        _ => Err("fittype: the model must be a library model name, an equation, a cell array of linear terms or an anonymous function".into()),
    }
}

fn no_spec(spec: &Spec, name: &str) -> Result<()> {
    if spec.independent.is_some()
        || spec.dependent.is_some()
        || spec.coefficients.is_some()
        || spec.problem.is_some()
    {
        return Err(format!(
            "fittype: library model '{name}' fixes its variable and coefficient names"
        ));
    }
    Ok(())
}

/// Identifiers of an expression in order of first appearance, with the
/// names called as functions.
fn walk(expr: &Expr, names: &mut Vec<String>, calls: &mut Vec<String>) -> Result<()> {
    match expr {
        Expr::Number(_)
        | Expr::Imaginary(_)
        | Expr::NameValue(_)
        | Expr::String(_)
        | Expr::DoubleQuoted(_) => {}
        Expr::Name(n) => {
            if !CONSTANTS.contains(&n.as_str()) && !names.contains(n) {
                names.push(n.clone());
            }
        }
        Expr::Call(n, args) => {
            if !calls.contains(n) {
                calls.push(n.clone());
            }
            for a in args {
                walk(a, names, calls)?;
            }
        }
        Expr::Unary(_, a) | Expr::Not(a) | Expr::Transpose(a) | Expr::NonConjugateTranspose(a) => {
            walk(a, names, calls)?
        }
        Expr::Binary(_, a, b) => {
            walk(a, names, calls)?;
            walk(b, names, calls)?;
        }
        Expr::Range(a, b, c) => {
            walk(a, names, calls)?;
            if let Some(b) = b {
                walk(b, names, calls)?;
            }
            walk(c, names, calls)?;
        }
        Expr::Matrix(rows) => {
            for e in rows.iter().flatten() {
                walk(e, names, calls)?;
            }
        }
        _ => {
            return Err(
                "fittype: the equation may use numbers, names, operators and function calls only"
                    .into(),
            );
        }
    }
    Ok(())
}

/// MATLAB fittype evaluates equations elementwise: *, / and ^ act as .*, ./ and .^.
pub(super) fn vectorize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev = ' ';
    let mut quoted = false;
    for c in text.chars() {
        if c == '\'' && !(prev.is_alphanumeric() || prev == ')' || prev == '_' || prev == '.') {
            quoted = !quoted;
        }
        if !quoted && matches!(c, '*' | '/' | '^' | '\\') && prev != '.' {
            out.push('.');
        }
        out.push(c);
        prev = c;
    }
    out
}

fn known_function(e: &mut Engine, name: &str) -> Result<bool> {
    Ok(crate::session::FUNCTIONS.contains(&name) || e.resolve_function(name)?.is_some())
}

fn handle_from_text(e: &mut Engine, params: &[String], body: &str) -> Result<FunctionHandle> {
    let text = format!("@({}) {}", params.join(","), body);
    let Expr::Anonymous(function) = parse_expression(&text)? else {
        return Err("fittype: the equation is not a valid expression".into());
    };
    match e.anonymous_handle(&function, false)? {
        Value::Container(Array::Handle(h)) => Ok(h),
        _ => Err("fittype: the equation is not a valid expression".into()),
    }
}

fn custom(e: &mut Engine, text: &str, spec: &Spec, _out: &mut Execution) -> Result<FitType> {
    let expr = parse_expression(text).map_err(|err| {
        format!("fittype: '{text}' is not a library model name or a valid equation ({err})")
    })?;
    let (mut names, mut calls) = (Vec::new(), Vec::new());
    walk(&expr, &mut names, &mut calls)?;
    for c in &calls {
        if !known_function(e, c)? {
            return Err(format!(
                "fittype: '{c}' in the equation '{text}' is not a function; coefficients are scalars and are not indexed"
            ));
        }
    }
    let indep = spec
        .independent
        .clone()
        .unwrap_or_else(|| vec!["x".to_owned()]);
    if indep.is_empty() || indep.len() > 2 {
        return Err("fittype: give one or two independent variables".into());
    }
    let dep = spec
        .dependent
        .clone()
        .unwrap_or_else(|| if indep.len() == 2 { "z" } else { "y" }.to_owned());
    let problems = spec.problem.clone().unwrap_or_default();
    for v in &indep {
        if !names.contains(v) {
            return Err(format!(
                "fittype: the independent variable '{v}' does not appear in the equation '{text}'; use it or name another independent variable with 'independent'"
            ));
        }
    }
    if names.contains(&dep) {
        return Err(format!(
            "fittype: the dependent variable '{dep}' cannot appear in the equation"
        ));
    }
    let coeffs: Vec<String> = match &spec.coefficients {
        Some(list) => {
            for n in &names {
                if !list.contains(n) && !problems.contains(n) && !indep.contains(n) {
                    return Err(format!(
                        "fittype: '{n}' in the equation is not a coefficient, problem parameter or independent variable"
                    ));
                }
            }
            list.clone()
        }
        None => names
            .iter()
            .filter(|n| !indep.contains(n) && !problems.contains(n))
            .cloned()
            .collect(),
    };
    if coeffs.is_empty() {
        return Err(format!(
            "fittype: the equation '{text}' has no coefficients"
        ));
    }
    let mut all: Vec<String> = coeffs.clone();
    all.extend(problems.iter().cloned());
    all.extend(indep.iter().cloned());
    all.push(dep.clone());
    check_names(&all)?;
    all.pop();
    let handle = handle_from_text(e, &all, &vectorize(text))?;
    Ok(FitType {
        kind: Kind::Custom(Custom {
            handle,
            order: (0..all.len()).collect(),
        }),
        coeffs,
        problems,
        indep,
        dep,
        formula: text.to_owned(),
    })
}

/// fittype({'term1', 'term2', ...}): y = a*term1 + b*term2 + ...
fn linear(e: &mut Engine, terms: &[String], spec: &Spec) -> Result<FitType> {
    if terms.is_empty() {
        return Err("fittype: a linear model needs at least one term".into());
    }
    let indep = spec
        .independent
        .clone()
        .unwrap_or_else(|| vec!["x".to_owned()]);
    if indep.is_empty() || indep.len() > 2 {
        return Err("fittype: give one or two independent variables".into());
    }
    let dep = spec
        .dependent
        .clone()
        .unwrap_or_else(|| if indep.len() == 2 { "z" } else { "y" }.to_owned());
    let problems = spec.problem.clone().unwrap_or_default();
    let coeffs: Vec<String> = match &spec.coefficients {
        Some(list) if list.len() != terms.len() => {
            return Err(format!(
                "fittype: {} coefficient names given for {} linear terms",
                list.len(),
                terms.len()
            ));
        }
        Some(list) => list.clone(),
        None => (0..terms.len())
            .map(|k| {
                if k < 26 {
                    ((b'a' + k as u8) as char).to_string()
                } else {
                    format!("c{}", k + 1)
                }
            })
            .collect(),
    };
    let mut params = problems.clone();
    params.extend(indep.iter().cloned());
    let mut all = coeffs.clone();
    all.extend(params.iter().cloned());
    all.push(dep.clone());
    check_names(&all)?;
    let mut handles = Vec::new();
    let mut parts = Vec::new();
    for (term, name) in terms.iter().zip(&coeffs) {
        let expr = parse_expression(term)
            .map_err(|err| format!("fittype: '{term}' is not a valid term ({err})"))?;
        let (mut names, mut calls) = (Vec::new(), Vec::new());
        walk(&expr, &mut names, &mut calls)?;
        for c in &calls {
            if !known_function(e, c)? {
                return Err(format!(
                    "fittype: '{c}' in the term '{term}' is not a function"
                ));
            }
        }
        for n in &names {
            if !params.contains(n) {
                return Err(format!(
                    "fittype: '{n}' in the term '{term}' is not a problem parameter or independent variable"
                ));
            }
        }
        handles.push(handle_from_text(e, &params, &vectorize(term))?);
        let t = term.trim();
        parts.push(if t == "1" {
            name.clone()
        } else if names.len() <= 1 && !t.contains(['+', '-', ' ']) {
            format!("{name}*{t}")
        } else {
            format!("{name}*({t})")
        });
    }
    Ok(FitType {
        kind: Kind::Linear(Linear { handles }),
        coeffs,
        problems,
        indep,
        dep,
        formula: parts.join(" + "),
    })
}

/// fittype(@(a, b, x) ...): the last parameter (or the named ones) is independent.
fn from_handle(_e: &mut Engine, h: &FunctionHandle, spec: &Spec) -> Result<FitType> {
    let Some((function, _)) = h.anonymous_parts() else {
        return Err("fittype: a function handle model must be an anonymous function such as @(a,b,x) a*exp(b*x)".into());
    };
    let params = function.params.clone();
    if params.len() < 2 {
        return Err("fittype: the anonymous function needs at least one coefficient and one independent variable".into());
    }
    let indep = match &spec.independent {
        Some(v) => v.clone(),
        None => vec![params.last().unwrap().clone()],
    };
    if indep.is_empty() || indep.len() > 2 {
        return Err("fittype: give one or two independent variables".into());
    }
    let problems = spec.problem.clone().unwrap_or_default();
    for n in indep.iter().chain(&problems) {
        if !params.contains(n) {
            return Err(format!(
                "fittype: '{n}' is not an input of the anonymous function"
            ));
        }
    }
    let coeffs: Vec<String> = match &spec.coefficients {
        Some(list) => list.clone(),
        None => params
            .iter()
            .filter(|p| !indep.contains(p) && !problems.contains(p))
            .cloned()
            .collect(),
    };
    let dep = spec
        .dependent
        .clone()
        .unwrap_or_else(|| if indep.len() == 2 { "z" } else { "y" }.to_owned());
    let mut all = coeffs.clone();
    all.extend(problems.iter().cloned());
    all.extend(indep.iter().cloned());
    if all.len() != params.len() || params.iter().any(|p| !all.contains(p)) {
        return Err("fittype: every input of the anonymous function must be a coefficient, problem parameter or independent variable".into());
    }
    if coeffs.is_empty() {
        return Err("fittype: the anonymous function has no coefficients".into());
    }
    let order = params
        .iter()
        .map(|p| all.iter().position(|a| a == p).unwrap())
        .collect();
    let text = &function.text;
    let body = text
        .find(')')
        .map(|k| text[k + 1..].trim().to_owned())
        .unwrap_or_else(|| text.clone());
    Ok(FitType {
        kind: Kind::Custom(Custom {
            handle: h.clone(),
            order,
        }),
        coeffs,
        problems,
        indep,
        dep,
        formula: body,
    })
}

fn column(data: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        data.len(),
        1,
        data.to_vec(),
    )?))
}

/// Real values from a model function result: n values or one to broadcast.
fn real_values(v: Value, n: usize, what: &str) -> Result<Vec<f64>> {
    let m = match v {
        Value::Matrix(m) => m,
        Value::Logical(l) => l.to_numeric(&std::sync::atomic::AtomicBool::new(false))?,
        Value::Complex(_) => {
            return Err(format!(
                "Complex values computed by the {what}; fitting needs real values (check the data domain or tighten Lower and Upper bounds)"
            ));
        }
        _ => return Err(format!("The {what} must return real numeric values")),
    };
    if m.len() == n {
        Ok(m.data().to_vec())
    } else if m.len() == 1 {
        Ok(vec![m.data()[0]; n])
    } else {
        Err(format!(
            "The {what} returned {} values for {n} points; write the equation elementwise (.*, ./, .^)",
            m.len()
        ))
    }
}

/// Model values at the points `xs` (one slice per independent variable, in
/// the model's own, possibly normalized, coordinates).
pub(super) fn values(
    e: &mut Engine,
    ft: &FitType,
    coef: &[f64],
    problem: &[f64],
    knots: Option<&Knots>,
    xs: &[&[f64]],
    out: &mut Execution,
) -> Result<Vec<f64>> {
    let n = xs[0].len();
    match &ft.kind {
        Kind::Library(lib) => {
            let y = xs.get(1).copied();
            Ok((0..n)
                .map(|i| lib.value(coef, xs[0][i], y.map_or(0., |y| y[i])))
                .collect())
        }
        Kind::Custom(c) => {
            let mut all: Vec<Value> = coef
                .iter()
                .map(|v| Value::Matrix(Matrix::scalar(*v)))
                .collect();
            all.extend(problem.iter().map(|v| Value::Matrix(Matrix::scalar(*v))));
            for x in xs {
                all.push(column(x)?);
            }
            let args = c.order.iter().map(|&k| all[k].clone()).collect();
            let mut v = e.call_handle(&c.handle, args, 1, None, out)?;
            if v.is_empty() {
                return Err("The model function returned no value".into());
            }
            real_values(v.swap_remove(0), n, "model function")
        }
        Kind::Linear(_) => {
            let basis = linear_basis(e, ft, problem, xs, out)?;
            Ok((0..n)
                .map(|i| (0..coef.len()).map(|k| coef[k] * basis[(i, k)]).sum())
                .collect())
        }
        Kind::Interp(m) => {
            let Some(k) = knots else {
                return Err("This interpolant has no data; create it with fit".into());
            };
            if let Some(s) = &k.smooth {
                return Ok(super::smooth::values(s, xs[0]));
            }
            let args = vec![
                column(&k.x)?,
                column(&k.y)?,
                column(xs[0])?,
                Value::Text(m.interp1().into()),
                Value::Text("extrap".into()),
            ];
            let v = e.polyopt_interp1(&args)?;
            real_values(v, n, "interpolant")
        }
    }
}

/// Columns of a linear model (library polynomial or custom linear terms).
pub(super) fn linear_basis(
    e: &mut Engine,
    ft: &FitType,
    problem: &[f64],
    xs: &[&[f64]],
    out: &mut Execution,
) -> Result<DMatrix<f64>> {
    let n = xs[0].len();
    let p = ft.coeffs.len();
    let mut basis = DMatrix::zeros(n, p);
    match &ft.kind {
        Kind::Library(lib) => {
            let mut g = vec![0.; p];
            let zero = vec![0.; p];
            for i in 0..n {
                lib.gradient(&zero, xs[0][i], xs.get(1).map_or(0., |y| y[i]), &mut g);
                for k in 0..p {
                    basis[(i, k)] = g[k];
                }
            }
        }
        Kind::Linear(l) => {
            for (k, h) in l.handles.iter().enumerate() {
                let mut args: Vec<Value> = problem
                    .iter()
                    .map(|v| Value::Matrix(Matrix::scalar(*v)))
                    .collect();
                for x in xs {
                    args.push(column(x)?);
                }
                let mut v = e.call_handle(h, args, 1, None, out)?;
                if v.is_empty() {
                    return Err("A linear model term returned no value".into());
                }
                let col = real_values(v.swap_remove(0), n, "linear model term")?;
                for i in 0..n {
                    basis[(i, k)] = col[i];
                }
            }
        }
        _ => return Err("This model is not linear in its coefficients".into()),
    }
    Ok(basis)
}

/// Derivatives of the model values with respect to the coefficients (n-by-p)
/// and the number of model evaluations they took. Library and linear models
/// are exact; custom equations use central differences of step
/// eps^(1/3)*max(|b|, 1), clipped to [diff_min, diff_max] and kept inside the bounds.
#[allow(clippy::too_many_arguments)]
pub(super) fn coefficient_jacobian(
    e: &mut Engine,
    ft: &FitType,
    coef: &[f64],
    problem: &[f64],
    xs: &[&[f64]],
    bounds: Option<(&[f64], &[f64])>,
    diff: (f64, f64),
    out: &mut Execution,
) -> Result<(DMatrix<f64>, usize)> {
    let n = xs[0].len();
    let p = coef.len();
    match &ft.kind {
        Kind::Library(lib) => {
            let mut j = DMatrix::zeros(n, p);
            let mut g = vec![0.; p];
            for i in 0..n {
                lib.gradient(coef, xs[0][i], xs.get(1).map_or(0., |y| y[i]), &mut g);
                for k in 0..p {
                    j[(i, k)] = g[k];
                }
            }
            Ok((j, 0))
        }
        Kind::Linear(_) => Ok((linear_basis(e, ft, problem, xs, out)?, 1)),
        Kind::Custom(_) => {
            let mut j = DMatrix::zeros(n, p);
            let mut count = 0;
            for k in 0..p {
                e.check_cancel()?;
                let mut h = (f64::EPSILON.cbrt() * coef[k].abs().max(1.)).clamp(diff.0, diff.1);
                let (lo, hi) =
                    bounds.map_or((f64::NEG_INFINITY, f64::INFINITY), |(l, u)| (l[k], u[k]));
                let (mut up, mut dn) = (coef.to_vec(), coef.to_vec());
                let (fu, fd, span);
                if coef[k] + h <= hi && coef[k] - h >= lo {
                    up[k] += h;
                    dn[k] -= h;
                    fu = values(e, ft, &up, problem, None, xs, out)?;
                    fd = values(e, ft, &dn, problem, None, xs, out)?;
                    span = 2. * h;
                    count += 2;
                } else {
                    if coef[k] + h > hi {
                        h = -h;
                    }
                    up[k] += h;
                    fu = values(e, ft, &up, problem, None, xs, out)?;
                    fd = values(e, ft, coef, problem, None, xs, out)?;
                    span = h;
                    count += 2;
                }
                for i in 0..n {
                    j[(i, k)] = (fu[i] - fd[i]) / span;
                }
            }
            Ok((j, count))
        }
        Kind::Interp(_) => Ok((DMatrix::zeros(n, p), 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equations_are_vectorized_once() {
        assert_eq!(vectorize("a*x^2+b/x"), "a.*x.^2+b./x");
        assert_eq!(vectorize("a.*x.^2"), "a.*x.^2");
        assert_eq!(vectorize("2.5*x'"), "2.5.*x'");
    }
}
