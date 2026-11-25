//! Optimize: sets up one optimization problem and solves it with the
//! engine's solvers (fminunc, fminsearch, fminbnd, fmincon, fzero, fsolve,
//! linprog, intlinprog, quadprog, lsqnonlin, lsqcurvefit, ga, particleswarm,
//! patternsearch, simulannealbnd). Run applies the inputs: the script defines
//! the typed data, the non-default options and the solver call, and Export
//! and Generate script use that same text. The solvers have no OutputFcn,
//! so the private probe solves once more with an objective that records
//! every evaluation: the progress chart shows the best point after each
//! function evaluation and the map shows where the solver went.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::Array;

// ---------------------------------------------------------------------------
// Problem, constraints and solvers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Problem {
    Minimize,
    Equations,
    Linear,
    MixedInteger,
    Quadratic,
    LeastSquares,
    CurveFit,
    Global,
}
impl Problem {
    pub const ALL: [Self; 8] = [
        Self::Minimize,
        Self::Equations,
        Self::Linear,
        Self::MixedInteger,
        Self::Quadratic,
        Self::LeastSquares,
        Self::CurveFit,
        Self::Global,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize a function",
            Self::Equations => "Solve equations",
            Self::Linear => "Linear program",
            Self::MixedInteger => "Mixed-integer linear program",
            Self::Quadratic => "Quadratic program",
            Self::LeastSquares => "Nonlinear least squares",
            Self::CurveFit => "Curve fitting",
            Self::Global => "Global minimum",
        }
    }
    /// Whether the problem can have constraints of this kind.
    pub fn allows(self, kind: Constraint) -> bool {
        match self {
            Self::Minimize | Self::Global => true,
            Self::Linear | Self::MixedInteger | Self::Quadratic => kind != Constraint::Nonlinear,
            Self::LeastSquares | Self::CurveFit => kind == Constraint::Bounds,
            Self::Equations => false,
        }
    }
    /// The objective is a function of x rather than coefficient matrices.
    pub fn has_function(self) -> bool {
        !matches!(self, Self::Linear | Self::MixedInteger | Self::Quadratic)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Constraint {
    Bounds,
    Inequalities,
    Equalities,
    Nonlinear,
}
impl Constraint {
    pub const ALL: [Self; 4] = [
        Self::Bounds,
        Self::Inequalities,
        Self::Equalities,
        Self::Nonlinear,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bounds => "Bounds  lb ≤ x ≤ ub",
            Self::Inequalities => "Linear inequalities  A·x ≤ b",
            Self::Equalities => "Linear equalities  Aeq·x = beq",
            Self::Nonlinear => "Nonlinear  c(x) ≤ 0, ceq(x) = 0",
        }
    }
}

/// The constraint kinds a problem has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Constraints {
    pub bounds: bool,
    pub inequalities: bool,
    pub equalities: bool,
    pub nonlinear: bool,
}
impl Constraints {
    pub fn get(self, kind: Constraint) -> bool {
        match kind {
            Constraint::Bounds => self.bounds,
            Constraint::Inequalities => self.inequalities,
            Constraint::Equalities => self.equalities,
            Constraint::Nonlinear => self.nonlinear,
        }
    }
    pub fn set(&mut self, kind: Constraint, on: bool) {
        match kind {
            Constraint::Bounds => self.bounds = on,
            Constraint::Inequalities => self.inequalities = on,
            Constraint::Equalities => self.equalities = on,
            Constraint::Nonlinear => self.nonlinear = on,
        }
    }
    /// Only the kinds `problem` allows.
    pub fn within(self, problem: Problem) -> Self {
        let mut kept = Self::default();
        for kind in Constraint::ALL {
            kept.set(kind, self.get(kind) && problem.allows(kind));
        }
        kept
    }
    /// Constraints beyond bounds.
    pub fn general(self) -> bool {
        self.inequalities || self.equalities || self.nonlinear
    }
    pub fn any(self) -> bool {
        self.bounds || self.general()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Solver {
    Fminunc,
    Fminsearch,
    Fminbnd,
    Fmincon,
    Fzero,
    Fsolve,
    Linprog,
    Intlinprog,
    Quadprog,
    Lsqnonlin,
    Lsqcurvefit,
    Ga,
    Particleswarm,
    Patternsearch,
    Simulannealbnd,
}
impl Solver {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fminunc => "fminunc",
            Self::Fminsearch => "fminsearch",
            Self::Fminbnd => "fminbnd",
            Self::Fmincon => "fmincon",
            Self::Fzero => "fzero",
            Self::Fsolve => "fsolve",
            Self::Linprog => "linprog",
            Self::Intlinprog => "intlinprog",
            Self::Quadprog => "quadprog",
            Self::Lsqnonlin => "lsqnonlin",
            Self::Lsqcurvefit => "lsqcurvefit",
            Self::Ga => "ga",
            Self::Particleswarm => "particleswarm",
            Self::Patternsearch => "patternsearch",
            Self::Simulannealbnd => "simulannealbnd",
        }
    }
    /// fminsearch, fminbnd and fzero: MatRust gives x, fval and exitflag
    /// only and runs them with their built-in tolerances (no options).
    pub fn plain(self) -> bool {
        matches!(self, Self::Fminsearch | Self::Fminbnd | Self::Fzero)
    }
    /// Draws random numbers: the script seeds the generator first.
    pub fn stochastic(self) -> bool {
        matches!(self, Self::Ga | Self::Particleswarm | Self::Simulannealbnd)
    }
    /// Takes the number of variables rather than a start point.
    pub fn takes_nvars(self) -> bool {
        matches!(self, Self::Ga | Self::Particleswarm)
    }
    /// Reads the start point x0.
    pub fn takes_x0(self) -> bool {
        !matches!(
            self,
            Self::Fminbnd
                | Self::Linprog
                | Self::Intlinprog
                | Self::Quadprog
                | Self::Ga
                | Self::Particleswarm
        )
    }
    /// Its second output is the sum of squares; exitflag and output follow
    /// the residual.
    pub fn least_squares(self) -> bool {
        matches!(self, Self::Lsqnonlin | Self::Lsqcurvefit)
    }
    /// Returns Lagrange multipliers.
    pub fn has_lambda(self) -> bool {
        matches!(self, Self::Fmincon | Self::Linprog | Self::Quadprog)
    }
    /// Solves for coefficient matrices rather than a function.
    pub fn linear(self) -> bool {
        matches!(self, Self::Linprog | Self::Intlinprog | Self::Quadprog)
    }
    /// The common options of the solver (Display and Algorithm aside).
    pub fn options(self) -> &'static [Opt] {
        use Opt::*;
        match self {
            Self::Fminunc => &[
                MaxIterations,
                MaxFunctionEvaluations,
                OptimalityTolerance,
                StepTolerance,
            ],
            Self::Fsolve | Self::Lsqnonlin | Self::Lsqcurvefit => &[
                MaxIterations,
                MaxFunctionEvaluations,
                OptimalityTolerance,
                StepTolerance,
                FunctionTolerance,
            ],
            Self::Fmincon => &[
                MaxIterations,
                MaxFunctionEvaluations,
                OptimalityTolerance,
                StepTolerance,
                ConstraintTolerance,
            ],
            Self::Linprog | Self::Quadprog => {
                &[MaxIterations, OptimalityTolerance, ConstraintTolerance]
            }
            Self::Intlinprog => &[ConstraintTolerance],
            Self::Ga => &[MaxGenerations, FunctionTolerance, ConstraintTolerance],
            Self::Particleswarm => &[MaxIterations, FunctionTolerance],
            Self::Simulannealbnd => &[MaxIterations, MaxFunctionEvaluations, FunctionTolerance],
            Self::Patternsearch => &[
                MaxIterations,
                MaxFunctionEvaluations,
                StepTolerance,
                FunctionTolerance,
                ConstraintTolerance,
            ],
            Self::Fminsearch | Self::Fminbnd | Self::Fzero => &[],
        }
    }
    /// Algorithms MatRust really has for the solver; the first is the default.
    pub fn algorithms(self) -> &'static [&'static str] {
        match self {
            Self::Fsolve => &["trust-region-dogleg", "levenberg-marquardt"],
            _ => &[],
        }
    }
}

/// Solvers valid for a problem, its constraints and its size; the first is
/// the recommended one. `scalar`: the start point has one value.
pub fn solvers(
    problem: Problem,
    constraints: Constraints,
    system: bool,
    scalar: bool,
) -> Vec<Solver> {
    use Solver::*;
    let c = constraints.within(problem);
    match problem {
        Problem::Minimize if c.general() => vec![Fmincon],
        Problem::Minimize if c.bounds && scalar => vec![Fminbnd, Fmincon],
        Problem::Minimize if c.bounds => vec![Fmincon],
        Problem::Minimize => vec![Fminunc, Fminsearch, Fmincon],
        Problem::Equations if system => vec![Fsolve],
        Problem::Equations => vec![Fzero, Fsolve],
        Problem::Linear => vec![Linprog],
        Problem::MixedInteger => vec![Intlinprog],
        Problem::Quadratic => vec![Quadprog],
        Problem::LeastSquares => vec![Lsqnonlin],
        Problem::CurveFit => vec![Lsqcurvefit],
        Problem::Global if c.general() => vec![Ga, Patternsearch],
        Problem::Global if c.bounds => vec![Particleswarm, Ga, Patternsearch, Simulannealbnd],
        Problem::Global => vec![Patternsearch, Ga, Particleswarm, Simulannealbnd],
    }
}

/// Common numeric options, by their optimoptions names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opt {
    MaxIterations,
    MaxGenerations,
    MaxFunctionEvaluations,
    OptimalityTolerance,
    StepTolerance,
    FunctionTolerance,
    ConstraintTolerance,
}
impl Opt {
    pub const ALL: [Self; 7] = [
        Self::MaxIterations,
        Self::MaxGenerations,
        Self::MaxFunctionEvaluations,
        Self::OptimalityTolerance,
        Self::StepTolerance,
        Self::FunctionTolerance,
        Self::ConstraintTolerance,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::MaxIterations => "MaxIterations",
            Self::MaxGenerations => "MaxGenerations",
            Self::MaxFunctionEvaluations => "MaxFunctionEvaluations",
            Self::OptimalityTolerance => "OptimalityTolerance",
            Self::StepTolerance => "StepTolerance",
            Self::FunctionTolerance => "FunctionTolerance",
            Self::ConstraintTolerance => "ConstraintTolerance",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::MaxIterations => "Max iterations",
            Self::MaxGenerations => "Max generations",
            Self::MaxFunctionEvaluations => "Max function evaluations",
            Self::OptimalityTolerance => "Optimality tolerance",
            Self::StepTolerance => "Step tolerance",
            Self::FunctionTolerance => "Function tolerance",
            Self::ConstraintTolerance => "Constraint tolerance",
        }
    }
    /// A count: a nonnegative whole number or Inf.
    pub fn count(self) -> bool {
        matches!(
            self,
            Self::MaxIterations | Self::MaxGenerations | Self::MaxFunctionEvaluations
        )
    }
    fn index(self) -> usize {
        Self::ALL.iter().position(|o| *o == self).unwrap_or(0)
    }
}

/// MatRust's default of an option for `n` variables (None: depends on n
/// and n is not known).
pub fn default_value(solver: Solver, opt: Opt, n: Option<usize>) -> Option<f64> {
    use Opt::*;
    use Solver::*;
    let per = |k: f64| n.map(|n| k * n as f64);
    match (solver, opt) {
        (Fmincon, MaxIterations) => Some(400.),
        (Fmincon, MaxFunctionEvaluations) => Some(3000.),
        (Fmincon, StepTolerance) => Some(1e-10),
        (Fminunc | Fsolve | Lsqnonlin | Lsqcurvefit | Linprog | Quadprog, MaxIterations) => {
            Some(400.)
        }
        (Fminunc | Fsolve | Lsqnonlin | Lsqcurvefit, MaxFunctionEvaluations) => per(100.),
        (Patternsearch, MaxIterations) => per(100.),
        (Patternsearch, MaxFunctionEvaluations) => per(2000.),
        (Particleswarm, MaxIterations) => per(200.),
        (Ga, MaxGenerations) => per(100.),
        (Simulannealbnd, MaxIterations) => Some(f64::INFINITY),
        (Simulannealbnd, MaxFunctionEvaluations) => per(3000.),
        (_, OptimalityTolerance | StepTolerance | FunctionTolerance | ConstraintTolerance) => {
            Some(1e-6)
        }
        _ => None,
    }
}
/// The default as the option field's hint.
fn default_hint(solver: Solver, opt: Opt, n: Option<usize>) -> String {
    use Opt::*;
    use Solver::*;
    if let Some(v) = default_value(solver, opt, n) {
        return format!("Default {}", literal(v));
    }
    let k = match (solver, opt) {
        (Patternsearch | Ga, _) if opt != MaxFunctionEvaluations => 100,
        (Patternsearch, _) => 2000,
        (Particleswarm, _) => 200,
        (Simulannealbnd, _) => 3000,
        _ => 100,
    };
    format!("Default {k} × variables")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayLevel {
    Off,
    Final,
    Iter,
    Notify,
}
impl DisplayLevel {
    pub const ALL: [Self; 4] = [Self::Off, Self::Final, Self::Iter, Self::Notify];
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Final => "final",
            Self::Iter => "iter",
            Self::Notify => "notify",
        }
    }
}

/// Where a function comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Typed text of x, made an anonymous function.
    Expression,
    /// A function handle variable.
    Workspace,
    /// A function file on the path, by name.
    File,
}
impl Source {
    pub const ALL: [Self; 3] = [Self::Expression, Self::Workspace, Self::File];
    pub fn label(self) -> &'static str {
        match self {
            Self::Expression => "Expression",
            Self::Workspace => "Workspace",
            Self::File => "Function file",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub source: Source,
    pub expression: String,
    pub variable: String,
    pub file: String,
}
impl Function {
    fn typed(expression: &str) -> Self {
        Self {
            source: Source::Expression,
            expression: expression.into(),
            variable: String::new(),
            file: String::new(),
        }
    }
}

/// Data inputs: typed values or workspace variable names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    X0,
    F,
    H,
    Intcon,
    Xdata,
    Ydata,
    Lb,
    Ub,
    A,
    B,
    Aeq,
    Beq,
}
impl Slot {
    pub const COUNT: usize = 12;
    /// The script's variable name for a typed value.
    pub fn base(self) -> &'static str {
        match self {
            Self::X0 => "x0",
            Self::F => "f",
            Self::H => "H",
            Self::Intcon => "intcon",
            Self::Xdata => "xdata",
            Self::Ydata => "ydata",
            Self::Lb => "lb",
            Self::Ub => "ub",
            Self::A => "A",
            Self::B => "b",
            Self::Aeq => "Aeq",
            Self::Beq => "beq",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::X0 => "Start point x0",
            Self::F => "Linear coefficients f",
            Self::H => "Quadratic matrix H",
            Self::Intcon => "Integer variables",
            Self::Xdata => "xdata",
            Self::Ydata => "ydata",
            Self::Lb => "Lower bounds lb",
            Self::Ub => "Upper bounds ub",
            Self::A => "A",
            Self::B => "b",
            Self::Aeq => "Aeq",
            Self::Beq => "beq",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

// ---------------------------------------------------------------------------
// Typed values

/// A number the engine reads back exactly, Inf and NaN included.
fn literal(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else {
        number(v)
    }
}

fn number_token(token: &str) -> Option<f64> {
    let (negative, body) = match token.as_bytes().first()? {
        b'-' => (true, &token[1..]),
        b'+' => (false, &token[1..]),
        _ => (false, token),
    };
    let v = if body.eq_ignore_ascii_case("inf") {
        f64::INFINITY
    } else if body.eq_ignore_ascii_case("nan") {
        f64::NAN
    } else if body.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        body.parse::<f64>().ok().filter(|v| v.is_finite())?
    } else {
        return None;
    };
    Some(if negative { -v } else { v })
}

/// A typed numeric matrix: `[1 2; 3 4]`, `1 2 3`, `5` or `[]`, with Inf and
/// NaN. Rows, columns and the values row by row; None for anything else.
pub fn parse_matrix(text: &str) -> Option<(usize, usize, Vec<f64>)> {
    let text = text.trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(text);
    if inner.contains(['[', ']', '\n', '\r']) {
        return None;
    }
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for row in inner.split(';') {
        let values = row
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|t| !t.is_empty())
            .map(number_token)
            .collect::<Option<Vec<f64>>>()?;
        if !values.is_empty() {
            rows.push(values);
        }
    }
    let cols = rows.first().map_or(0, Vec::len);
    if rows.iter().any(|r| r.len() != cols) {
        return None;
    }
    Some((rows.len(), cols, rows.concat()))
}

/// The matrix as the script writes it.
fn matrix_text(rows: usize, cols: usize, values: &[f64]) -> String {
    if rows * cols == 1 {
        return literal(values[0]);
    }
    let lines: Vec<String> = (0..rows)
        .map(|r| {
            values[r * cols..(r + 1) * cols]
                .iter()
                .map(|v| literal(*v))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    format!("[{}]", lines.join("; "))
}

/// Names an expression reads (not fields, not inside text), in order.
pub fn identifiers(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut previous: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' {
            let transpose = c == '\''
                && previous.is_some_and(|p| {
                    p.is_alphanumeric() || matches!(p, '_' | ')' | ']' | '}' | '.' | '\'')
                });
            i += 1;
            if !transpose {
                while i < chars.len() {
                    if chars[i] == c {
                        if chars.get(i + 1) == Some(&c) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            previous = Some(')');
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                let sign = usize::from(matches!(chars.get(i + 1), Some('+' | '-')));
                if chars.get(i + 1 + sign).is_some_and(char::is_ascii_digit) {
                    i += 1 + sign;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            if i < chars.len() && matches!(chars[i], 'i' | 'j') {
                i += 1;
            }
            previous = Some('0');
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if previous != Some('.') && !out.contains(&word) {
                out.push(word);
            }
            previous = Some('a');
            continue;
        }
        if !c.is_whitespace() {
            previous = Some(c);
        }
        i += 1;
    }
    out
}

/// How a data input reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Empty,
    /// A workspace variable, by name.
    Variable(String),
    /// A typed matrix, as the script writes it.
    Literal {
        rows: usize,
        cols: usize,
        values: Vec<f64>,
    },
    /// Other typed text, defined as it is.
    Expression(String),
}
impl Data {
    pub fn resolve(text: &str, ws: &Snapshot) -> Self {
        let text = text.trim();
        if text.is_empty() {
            return Self::Empty;
        }
        if matrust::files::valid_name(text) && ws.contains(text) {
            return Self::Variable(text.to_owned());
        }
        match parse_matrix(text) {
            Some((0, _, _)) => Self::Empty,
            Some((rows, cols, values)) => Self::Literal { rows, cols, values },
            None => Self::Expression(text.to_owned()),
        }
    }
    /// Rows and columns when known before running.
    pub fn size(&self, ws: &Snapshot) -> Option<(usize, usize)> {
        match self {
            Self::Literal { rows, cols, .. } => Some((*rows, *cols)),
            Self::Variable(name) => ws.get(name).map(|a| {
                let dims = a.dims();
                (
                    dims.first().copied().unwrap_or(0),
                    dims.iter().skip(1).product(),
                )
            }),
            _ => None,
        }
    }
    pub fn numel(&self, ws: &Snapshot) -> Option<usize> {
        self.size(ws).map(|(r, c)| r * c)
    }
}

fn numeric(array: &Array) -> bool {
    matches!(
        array,
        Array::Numeric(_) | Array::Typed(_) | Array::Logical(_)
    ) && !array.is_complex()
}

// ---------------------------------------------------------------------------
// The inputs

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionSpec {
    /// None: the solver's default display.
    pub display: Option<DisplayLevel>,
    /// Typed values by `Opt::ALL` position; blank is the default.
    pub values: [String; 7],
    /// None: the solver's default algorithm.
    pub algorithm: Option<&'static str>,
}
impl OptionSpec {
    pub fn value(&self, opt: Opt) -> &str {
        &self.values[opt.index()]
    }
    pub fn set(&mut self, opt: Opt, text: &str) {
        self.values[opt.index()] = text.to_owned();
    }
}

/// Everything the user sets up; Run copies it to the applied inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub problem: Problem,
    /// Solve equations: a system rather than one equation in one unknown.
    pub system: bool,
    pub constraints: Constraints,
    /// The chosen solver; None (or one the problem rules out) follows the
    /// recommendation.
    pub solver: Option<Solver>,
    pub objective: Function,
    /// Nonlinear constraints: the expression source types c(x) here and
    /// ceq(x) in `ceq`.
    pub nonlcon: Function,
    pub ceq: String,
    pub data: [String; Slot::COUNT],
    /// ga and particleswarm: blank uses the size of x0.
    pub nvars: String,
    pub seed: String,
    pub options: OptionSpec,
}
impl Default for Spec {
    fn default() -> Self {
        let mut data: [String; Slot::COUNT] = Default::default();
        data[Slot::X0.index()] = "[-1 2]".into();
        Self {
            problem: Problem::Minimize,
            system: false,
            constraints: Constraints::default(),
            solver: None,
            objective: Function::typed("(x(1)-1)^2 + 100*(x(2)-x(1)^2)^2"),
            nonlcon: Function::typed(""),
            ceq: String::new(),
            data,
            nvars: String::new(),
            seed: "0".into(),
            options: OptionSpec::default(),
        }
    }
}
impl Spec {
    pub fn text(&self, slot: Slot) -> &str {
        &self.data[slot.index()]
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set(&mut self, slot: Slot, text: &str) {
        self.data[slot.index()] = text.to_owned();
    }
    pub fn constraints(&self) -> Constraints {
        self.constraints.within(self.problem)
    }
    /// The start point has one value.
    fn scalar(&self, ws: &Snapshot) -> bool {
        Data::resolve(self.text(Slot::X0), ws).numel(ws) == Some(1)
    }
    /// Valid solvers, the recommended one first.
    pub fn solvers(&self, ws: &Snapshot) -> Vec<Solver> {
        solvers(self.problem, self.constraints, self.system, self.scalar(ws))
    }
    /// The solver that runs: the chosen one if valid, else the recommended.
    pub fn solver(&self, ws: &Snapshot) -> Solver {
        let valid = self.solvers(ws);
        self.solver
            .filter(|s| valid.contains(s))
            .unwrap_or(valid[0])
    }
}

/// Output names in the order of `NAMES`.
pub const NAMES: [(&str, &str); 4] = [
    ("solution", "Solution name"),
    ("objectiveValue", "Objective value name"),
    ("exitflag", "Exit flag name"),
    ("output", "Output name"),
];

/// What the script and the probe share, checked.
struct Plan {
    solver: Solver,
    problem: Problem,
    constraints: Constraints,
    /// Lines defining the typed data, in order.
    definitions: Vec<String>,
    /// Names those lines create.
    defined: Vec<String>,
    /// Workspace variables read.
    inputs: Vec<String>,
    /// The objective as the call names it.
    fun: Option<String>,
    /// Each slot as the call names it; None when not used.
    slots: [Option<String>; Slot::COUNT],
    nonlcon: Option<String>,
    nvars: Option<usize>,
    /// Options variable and its line.
    options: Option<(String, String)>,
    seed: Option<String>,
    /// solution, objectiveValue, exitflag and output names.
    names: [String; 4],
    tolerance: f64,
}
impl Plan {
    fn slot(&self, slot: Slot) -> Option<&str> {
        self.slots[slot.index()].as_deref()
    }
    /// The solver's arguments in order; None is a skipped slot.
    fn arguments(&self, fun: Option<&str>) -> Vec<Option<String>> {
        use Slot::*;
        use Solver::*;
        let s = |slot: Slot| self.slot(slot).map(str::to_owned);
        let fun = fun.map(str::to_owned);
        let options = self.options.as_ref().map(|(name, _)| name.clone());
        let nvars = self.nvars.map(|n| n.to_string());
        let nonlcon = self.nonlcon.clone();
        match self.solver {
            Fminunc | Fsolve => vec![fun, s(X0), options],
            Fminsearch | Fzero => vec![fun, s(X0)],
            Fminbnd => vec![fun, s(Lb), s(Ub)],
            Fmincon | Patternsearch => vec![
                fun,
                s(X0),
                s(A),
                s(B),
                s(Aeq),
                s(Beq),
                s(Lb),
                s(Ub),
                nonlcon,
                options,
            ],
            Ga => vec![
                fun,
                nvars,
                s(A),
                s(B),
                s(Aeq),
                s(Beq),
                s(Lb),
                s(Ub),
                nonlcon,
                options,
            ],
            Linprog => vec![s(F), s(A), s(B), s(Aeq), s(Beq), s(Lb), s(Ub), options],
            Intlinprog => vec![
                s(F),
                s(Intcon),
                s(A),
                s(B),
                s(Aeq),
                s(Beq),
                s(Lb),
                s(Ub),
                options,
            ],
            Quadprog => vec![
                s(H),
                s(F),
                s(A),
                s(B),
                s(Aeq),
                s(Beq),
                s(Lb),
                s(Ub),
                None,
                options,
            ],
            Lsqnonlin => vec![fun, s(X0), s(Lb), s(Ub), options],
            Lsqcurvefit => vec![fun, s(X0), s(Xdata), s(Ydata), s(Lb), s(Ub), options],
            Particleswarm => vec![fun, nvars, s(Lb), s(Ub), options],
            Simulannealbnd => vec![fun, s(X0), s(Lb), s(Ub), options],
        }
    }
    /// `solver(args)` with trailing skipped slots dropped and `[]` for the
    /// skipped ones before a given one.
    fn call(&self, fun: Option<&str>) -> String {
        let mut arguments = self.arguments(fun);
        while arguments.last().is_some_and(Option::is_none) {
            arguments.pop();
        }
        let arguments: Vec<String> = arguments
            .into_iter()
            .map(|a| a.unwrap_or_else(|| "[]".into()))
            .collect();
        format!("{}({})", self.solver.name(), arguments.join(", "))
    }
    /// The left-hand side of the solver call.
    fn outputs(&self) -> String {
        let [x, value, flag, output] = &self.names;
        if self.solver.plain() {
            format!("[{x}, {value}, {flag}]")
        } else if self.solver.least_squares() {
            format!("[{x}, {value}, ~, {flag}, {output}]")
        } else {
            format!("[{x}, {value}, {flag}, {output}]")
        }
    }
    /// The names Export leaves in the workspace.
    fn exported(&self) -> Vec<String> {
        let outputs = if self.solver.plain() { 3 } else { 4 };
        let mut names: Vec<String> = self.names[..outputs].to_vec();
        names.extend(self.defined.iter().cloned());
        names
    }
}

fn check_text(text: &str, what: &str) -> Result<String, String> {
    let text = text.trim();
    if !text.is_empty() {
        super::single_expression(text).map_err(|e| format!("Optimize: {what}: {e}"))?;
    }
    Ok(text.to_owned())
}

/// A typed option value: blank, a number, or Inf for a count.
fn parse_option(opt: Opt, text: &str) -> Result<Option<f64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    super::single_expression(text).map_err(|e| format!("Optimize: {}: {e}", opt.label()))?;
    let v = number_token(text).filter(|v| !v.is_nan());
    match v {
        Some(v) if opt.count() && v >= 0. && (v.fract() == 0. || v == f64::INFINITY) => Ok(Some(v)),
        Some(v) if !opt.count() && v > 0. && v.is_finite() => Ok(Some(v)),
        _ if opt.count() => Err(format!(
            "Optimize: {} must be a whole number or Inf",
            opt.label()
        )),
        _ => Err(format!(
            "Optimize: {} must be a positive number",
            opt.label()
        )),
    }
}

/// The options the script sets: only values that differ from the default.
fn changed_options(
    solver: Solver,
    options: &OptionSpec,
    n: Option<usize>,
) -> Result<Vec<(&'static str, String)>, String> {
    let mut pairs = Vec::new();
    if solver.plain() {
        return Ok(pairs);
    }
    if let Some(display) = options.display
        && display != DisplayLevel::Final
    {
        pairs.push(("Display", format!("'{}'", display.name())));
    }
    if let Some(algorithm) = options.algorithm
        && solver.algorithms().first().is_some_and(|d| *d != algorithm)
        && solver.algorithms().contains(&algorithm)
    {
        pairs.push(("Algorithm", format!("'{algorithm}'")));
    }
    for opt in solver.options() {
        let Some(v) = parse_option(*opt, options.value(*opt))? else {
            continue;
        };
        if default_value(solver, *opt, n) == Some(v) {
            continue;
        }
        pairs.push((opt.name(), literal(v)));
    }
    Ok(pairs)
}

// ---------------------------------------------------------------------------
// Results

/// The evaluation log, as the progress chart and the map draw it.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// (evaluation, best value so far).
    pub best: Vec<[f64; 2]>,
    /// (evaluation, constraint violation of the best point); empty when
    /// unconstrained.
    pub violation: Vec<[f64; 2]>,
    /// The best points in the order they were found (one or two variables).
    pub path: Vec<Vec<f64>>,
    /// The function value at each path point (signed for fzero).
    pub path_values: Vec<f64>,
}

/// The objective over a grid around the start and the solution.
#[derive(Clone, Debug)]
pub struct Map {
    pub x: Vec<f64>,
    /// Empty for one variable.
    pub y: Vec<f64>,
    /// Row by row from the lowest y (one row for one variable).
    pub values: Vec<f64>,
    /// Points that break a constraint, in the same order.
    pub infeasible: Vec<bool>,
}
impl Map {
    pub fn two_d(&self) -> bool {
        !self.y.is_empty()
    }
}

/// lsqcurvefit's data and fitted model.
#[derive(Clone, Debug)]
pub struct FitView {
    pub data: Vec<[f64; 2]>,
    pub curve: Vec<[f64; 2]>,
}

/// One solve as the private run computed it.
#[derive(Clone, Debug)]
pub struct Solution {
    pub solver: Solver,
    pub constrained: bool,
    pub x: Vec<f64>,
    pub fval: Vec<f64>,
    pub exitflag: f64,
    pub iterations: Option<f64>,
    pub evaluations: Option<f64>,
    pub message: String,
    pub violation: Option<f64>,
    /// Nonempty multiplier fields.
    pub lambda: Vec<(&'static str, Vec<f64>)>,
    pub start: Vec<f64>,
    pub progress: Progress,
    pub map: Option<Map>,
    pub fit: Option<FitView>,
}

const LAMBDA: [&str; 6] = [
    "lower",
    "upper",
    "ineqlin",
    "eqlin",
    "ineqnonlin",
    "eqnonlin",
];

const PROBE: [&str; 21] = [
    "app_x",
    "app_fval",
    "app_flag",
    "app_iter",
    "app_fc",
    "app_msg",
    "app_cv",
    "app_x0",
    "app_E",
    "app_V",
    "app_keep",
    "app_gx",
    "app_gy",
    "app_gz",
    "app_gv",
    "app_xd",
    "app_yd",
    "app_xq",
    "app_yq",
    "app_tol",
    "app_lambda_all",
];

fn matrix(outputs: &Outputs, name: &str) -> Result<(usize, usize, Vec<f64>), String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok((m.rows(), m.cols(), m.data().to_vec())),
        Some(Array::Logical(m)) => Ok((
            m.rows(),
            m.cols(),
            m.data().iter().map(|b| f64::from(*b)).collect(),
        )),
        _ => Err(format!("Optimize: {name} is not a real array")),
    }
}
fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    matrix(outputs, name).map(|(_, _, v)| v)
}
fn scalar(outputs: &Outputs, name: &str) -> Result<f64, String> {
    values(outputs, name)?
        .first()
        .copied()
        .ok_or_else(|| format!("Optimize: {name} is empty"))
}
fn text(outputs: &Outputs, name: &str) -> Result<String, String> {
    match outputs.get(name) {
        Some(Array::Character(c)) => Ok(String::from_utf16_lossy(c.units())),
        Some(Array::Numeric(m)) if m.data().is_empty() => Ok(String::new()),
        _ => Err(format!("Optimize: {name} is not text")),
    }
}
fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

impl Solution {
    pub fn from_outputs(
        (solver, _, constraints): (Solver, Problem, Constraints),
        outputs: &Outputs,
    ) -> Result<Self, String> {
        let x = values(outputs, "app_x")?;
        let n = x.len();
        let (rows, cols, log) = matrix(outputs, "app_E")?;
        let violation = values(outputs, "app_V")?;
        let keep = values(outputs, "app_keep")?;
        let mut progress = Progress::default();
        if rows > 0 && cols == n + 1 {
            let at = |k: usize, j: usize| log[k + j * rows];
            let merit = |k: usize| {
                let v = at(k, n);
                if solver == Solver::Fzero { v.abs() } else { v }
            };
            let constrained = violation.len() == rows;
            let mut best: Option<(f64, f64)> = None;
            for k in 0..rows {
                if keep.get(k).is_some_and(|v| *v != 0.) {
                    let v = if constrained {
                        violation.get(k).copied().unwrap_or(f64::NAN)
                    } else {
                        0.
                    };
                    best = Some((merit(k), v));
                    if n <= 2 {
                        progress.path.push((0..n).map(|j| at(k, j)).collect());
                        progress.path_values.push(at(k, n));
                    }
                }
                if let Some((value, v)) = best {
                    let e = (k + 1) as f64;
                    if value.is_finite() {
                        progress.best.push([e, value]);
                    }
                    if constrained && v.is_finite() {
                        progress.violation.push([e, v]);
                    }
                }
            }
        }
        let gx = values(outputs, "app_gx")?;
        let gy = values(outputs, "app_gy")?;
        let (gr, gc, gz) = matrix(outputs, "app_gz")?;
        let gv = values(outputs, "app_gv")?;
        let tolerance = scalar(outputs, "app_tol")?;
        let map = (!gx.is_empty() && gc == gx.len() && gr == gy.len().max(1)).then(|| {
            // Column-major grid: rows follow y, columns follow x.
            let (rows, cols) = (gr, gc);
            let order = |k: usize| (k / cols) + (k % cols) * rows;
            Map {
                x: gx.clone(),
                y: gy.clone(),
                values: (0..rows * cols).map(|k| gz[order(k)]).collect(),
                infeasible: (0..rows * cols)
                    .map(|k| {
                        gv.get(order(k))
                            .is_some_and(|v| v.is_nan() || *v > tolerance)
                    })
                    .collect(),
            }
        });
        let xd = values(outputs, "app_xd")?;
        let yd = values(outputs, "app_yd")?;
        let xq = values(outputs, "app_xq")?;
        let yq = values(outputs, "app_yq")?;
        let fit = (!xd.is_empty() && xd.len() == yd.len()).then(|| FitView {
            data: xd
                .iter()
                .zip(&yd)
                .map(|(x, y)| [*x, *y])
                .filter(|p| p[0].is_finite() && p[1].is_finite())
                .collect(),
            curve: xq
                .iter()
                .zip(&yq)
                .map(|(x, y)| [*x, *y])
                .filter(|p| p[0].is_finite() && p[1].is_finite())
                .collect(),
        });
        let (lr, _, lambda_all) = matrix(outputs, "app_lambda_all")?;
        // One column per multiplier field: [count; values...].
        let lambda = LAMBDA
            .iter()
            .enumerate()
            .filter_map(|(j, name)| {
                let column = lambda_all.get(j * lr..(j + 1) * lr)?;
                let count = *column.first()? as usize;
                let vals = column.get(1..1 + count)?.to_vec();
                (!vals.is_empty()).then_some((*name, vals))
            })
            .collect();
        Ok(Self {
            solver,
            constrained: constraints.any(),
            x,
            fval: values(outputs, "app_fval")?,
            exitflag: scalar(outputs, "app_flag")?,
            iterations: finite(scalar(outputs, "app_iter")?),
            evaluations: finite(scalar(outputs, "app_fc")?).or_else(|| {
                // fminsearch, fminbnd and fzero report no output struct: the
                // recorded run counted the same evaluations.
                solver.plain().then_some(rows as f64)
            }),
            message: text(outputs, "app_msg")?,
            violation: finite(scalar(outputs, "app_cv")?),
            lambda,
            start: values(outputs, "app_x0")?,
            progress,
            map,
            fit,
        })
    }
    /// What the progress chart and the map draw.
    pub fn value_name(&self) -> &'static str {
        match self.solver {
            Solver::Fzero => "f(x)",
            Solver::Fsolve => "Sum of squares of F(x)",
            s if s.least_squares() => "Sum of squares",
            _ => "Objective",
        }
    }
    /// Label and text of the objective value.
    pub fn value_row(&self) -> (&'static str, String) {
        match self.solver {
            Solver::Fsolve => {
                let norm = self.fval.iter().map(|v| v * v).sum::<f64>().sqrt();
                ("‖F(x)‖", short(norm))
            }
            Solver::Fzero => ("f(x)", vector_text(&self.fval)),
            s if s.least_squares() => ("Sum of squares", vector_text(&self.fval)),
            _ => ("Objective", vector_text(&self.fval)),
        }
    }
}

/// Up to eight values, short.
pub fn vector_text(values: &[f64]) -> String {
    const SHOWN: usize = 8;
    match values {
        [] => "[]".into(),
        [v] => short(*v),
        _ => {
            let items: Vec<String> = values.iter().take(SHOWN).map(|v| short(*v)).collect();
            let more = if values.len() > SHOWN {
                format!(" … ({} values)", values.len())
            } else {
                String::new()
            };
            format!("[{}]{more}", items.join(" "))
        }
    }
}

/// What an exit flag says, in plain words.
pub fn exit_meaning(solver: Solver, flag: f64) -> &'static str {
    use Solver::*;
    let global = matches!(solver, Ga | Particleswarm | Patternsearch | Simulannealbnd);
    match (solver, flag as i64) {
        (Fzero, 1) => "Found a point where the function changes sign.",
        (Fzero, -3) => "The search met a NaN or Inf value.",
        (Fzero, -4) => "The search met a complex value.",
        (Fzero, -5) => "The search may have closed in on a singular point, not a root.",
        (Fzero, -6) => "No sign change was found; there may be no root nearby.",
        (Fminsearch, 1) => "Converged: the simplex is smaller than its tolerances.",
        (Fminbnd, 1) => "Converged: the bracket is smaller than its tolerance.",
        (Fminbnd | Lsqnonlin | Lsqcurvefit, -2) => {
            "The bounds are inconsistent (a lower bound is above its upper bound)."
        }
        (Fsolve, 1) => "Solved: the equations hold within the tolerance.",
        (Fsolve, 2) => "Stopped: the step became tiny; the equations may be solved.",
        (Fsolve, 3) => "Stopped: the residual stopped changing; the equations may be solved.",
        (Fsolve, -2) => "Not solved: the residual stays large; try another start point.",
        (Fsolve, -3) => "Not solved: the trust region became too small.",
        (Linprog | Quadprog, 1) => "Found the optimal solution.",
        (Intlinprog, 1) => "Found the optimal integer solution.",
        (Intlinprog, 2) => "Stopped early with an integer-feasible point.",
        (Intlinprog | Linprog | Quadprog | Fmincon | Ga | Patternsearch, -2) => {
            "No feasible point was found."
        }
        (Linprog | Quadprog | Intlinprog, -3) => "The problem is unbounded.",
        (Fminunc, -3) => "The objective fell below its limit; the problem may be unbounded.",
        (Patternsearch, 1) => "Converged: the mesh is smaller than its tolerance.",
        (_, 1) if global => "Stopped: the best value no longer changes by more than the tolerance.",
        (_, 2) if global => "Stopped: the step or the change in x is below its tolerance.",
        (_, 3) if global => "Stopped: the best value changed less than the tolerance.",
        (_, -5) if global => "Stopped at the time limit.",
        (_, 1) => "Converged: the first-order optimality measure is below its tolerance.",
        (_, 2) => "Stopped: the change in x is below the step tolerance.",
        (_, 3) => "Stopped: the change in the objective is below its tolerance.",
        (_, 4) => "Stopped: the search direction became tiny.",
        (_, 5) => "Stopped: the predicted decrease is below the tolerance.",
        (_, 0) => "Stopped at the iteration or evaluation limit; the result may not be optimal.",
        (_, -1) => "Stopped by an output or plot function.",
        _ => "See the solver's message.",
    }
}

// ---------------------------------------------------------------------------
// The app

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Progress,
    Map,
    Fit,
}
impl View {
    pub fn label(self) -> &'static str {
        match self {
            Self::Progress => "Progress",
            Self::Map => "Objective map",
            Self::Fit => "Data and fit",
        }
    }
}

pub struct Optimize {
    /// The inputs as edited.
    pub spec: Spec,
    /// The inputs of the last Run: what the script uses. None before the
    /// first Run and after Stop.
    pub applied: Option<Spec>,
    /// Run clicks: each one is a new private run.
    pub runs: u64,
    pub stopped: bool,
    /// The host's run is going (set before each frame).
    pub running: bool,
    pub names: [String; 4],
    pub edited: [bool; 4],
    pub view: View,
    pub log_scale: bool,
    /// Last good result; errors keep it on screen.
    pub result: Option<Solution>,
    /// Workspace variables the applied script reads (from `prepare`).
    inputs: Vec<String>,
    /// Solver, problem and constraints of the applied script (from `prepare`).
    planned: Option<(Solver, Problem, Constraints)>,
}
impl Default for Optimize {
    fn default() -> Self {
        Self {
            spec: Spec::default(),
            applied: None,
            runs: 0,
            stopped: false,
            running: false,
            names: NAMES.map(|(base, _)| base.to_owned()),
            edited: [false; 4],
            view: View::Progress,
            log_scale: true,
            result: None,
            inputs: Vec::new(),
            planned: None,
        }
    }
}

impl Optimize {
    /// Run: the edited inputs become the applied ones and solve again.
    pub fn run(&mut self) {
        self.applied = Some(self.spec.clone());
        self.runs += 1;
        self.stopped = false;
    }
    /// Stop: cancels the running solve (the host stops a run whose script
    /// went away); the last result stays on screen.
    pub fn stop(&mut self) {
        if self.applied.take().is_some() {
            self.stopped = true;
        }
    }
    /// Output names: the user's, else defaults that replace no variable.
    pub fn names_in_use(&self, ws: &Snapshot) -> [String; 4] {
        let mut names: [String; 4] = Default::default();
        for k in 0..4 {
            names[k] = if self.edited[k] {
                self.names[k].trim().to_owned()
            } else {
                let others: Vec<&String> = (0..4)
                    .filter(|j| *j < k || (*j > k && self.edited[*j]))
                    .map(|j| if j < k { &names[j] } else { &self.names[j] })
                    .collect();
                unused_name(NAMES[k].0, |n| {
                    ws.contains(n) || others.iter().any(|o| o.trim() == n)
                })
            };
        }
        names
    }
    /// The output names, checked.
    pub fn output_names(&self, ws: &Snapshot, inputs: &[String]) -> Result<[String; 4], String> {
        let names = self.names_in_use(ws);
        for (k, name) in names.iter().enumerate() {
            let what = NAMES[k].1.trim_end_matches(" name").to_ascii_lowercase();
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Optimize: the {what} name is not a valid variable name"
                ));
            }
            if inputs.contains(name) {
                return Err(format!(
                    "Optimize: the {what} name {name} is an input of the problem"
                ));
            }
            if names[..k].contains(name) {
                return Err("Optimize: the output names must differ".into());
            }
        }
        Ok(names)
    }

    /// The applied inputs, checked and named.
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let Some(spec) = &self.applied else {
            return Err(if self.stopped {
                "Optimize: stopped. Press Run to solve again.".into()
            } else {
                "Optimize: set up the problem in INPUTS, then press Run.".into()
            });
        };
        build(spec, self, ws)
    }
}

/// Checks `spec` and names everything its script defines.
fn build(spec: &Spec, app: &Optimize, ws: &Snapshot) -> Result<Plan, String> {
    use Slot::*;
    let solver = spec.solver(ws);
    let problem = spec.problem;
    let constraints = spec.constraints();
    let mut inputs: Vec<String> = Vec::new();
    // Which slots the call reads.
    let mut used: Vec<Slot> = Vec::new();
    if solver.takes_x0() {
        used.push(X0);
    }
    match problem {
        Problem::Linear => used.push(F),
        Problem::MixedInteger => used.extend([F, Intcon]),
        Problem::Quadratic => used.extend([H, F]),
        Problem::CurveFit => used.extend([Xdata, Ydata]),
        _ => {}
    }
    if constraints.bounds {
        used.extend([Lb, Ub]);
    }
    if constraints.inequalities {
        used.extend([A, B]);
    }
    if constraints.equalities {
        used.extend([Aeq, Beq]);
    }
    let mut data: Vec<(Slot, Data)> = Vec::new();
    for slot in used {
        let resolved = Data::resolve(spec.text(slot), ws);
        match &resolved {
            Data::Variable(name) => {
                if !ws.get(name).is_some_and(numeric) {
                    return Err(format!(
                        "Optimize: {}: workspace variable {name} is not a real numeric array",
                        slot.label()
                    ));
                }
                add_input(&mut inputs, name);
            }
            Data::Expression(text) => {
                check_text(text, slot.label())?;
                read_inputs(&mut inputs, text, &[], ws);
            }
            _ => {}
        }
        data.push((slot, resolved));
    }
    let get = |slot: Slot| {
        data.iter()
            .find(|(s, _)| *s == slot)
            .map_or(&Data::Empty, |(_, d)| d)
    };
    let given = |slot: Slot| *get(slot) != Data::Empty;
    let numel = |slot: Slot| get(slot).numel(ws);
    // Required inputs.
    let need = |slot: Slot, why: &str| -> Result<(), String> {
        if given(slot) {
            Ok(())
        } else {
            Err(format!("Optimize: type {} ({why})", slot.label()))
        }
    };
    if solver.takes_x0() {
        need(X0, "a start point, for example [0 0]")?;
    }
    match problem {
        Problem::Linear | Problem::MixedInteger => need(F, "the objective is f'*x")?,
        Problem::Quadratic => need(H, "the objective is 0.5*x'*H*x + f'*x")?,
        Problem::CurveFit => {
            need(Xdata, "the data the model reads")?;
            need(Ydata, "the data the model should match")?;
        }
        _ => {}
    }
    if problem == Problem::MixedInteger {
        need(
            Intcon,
            "the indices of the integer variables, for example [1 2]",
        )?;
    }
    if constraints.bounds && !given(Lb) && !given(Ub) {
        return Err("Optimize: type lb or ub, or turn Bounds off".into());
    }
    if solver == Solver::Fminbnd {
        need(Lb, "fminbnd searches between lb and ub")?;
        need(Ub, "fminbnd searches between lb and ub")?;
    }
    for (lhs, rhs, kind, on) in [
        (A, B, "Linear inequalities", constraints.inequalities),
        (Aeq, Beq, "Linear equalities", constraints.equalities),
    ] {
        if on && (!given(lhs) || !given(rhs)) {
            return Err(format!(
                "Optimize: type {} and {}, or turn {kind} off",
                lhs.label(),
                rhs.label()
            ));
        }
    }
    // The number of variables, where known.
    let nvars = if solver.takes_nvars() {
        let typed = spec.nvars.trim();
        if typed.is_empty() {
            let sized = Data::resolve(spec.text(X0), ws)
                .numel(ws)
                .filter(|n| *n > 0);
            Some(sized.ok_or(
                "Optimize: type the number of variables (or a start point x0 to size them)",
            )?)
        } else {
            match typed.parse::<usize>() {
                Ok(n) if (1..=10_000).contains(&n) => Some(n),
                _ => {
                    return Err(
                        "Optimize: the number of variables must be a whole number from 1 to 10000"
                            .into(),
                    );
                }
            }
        }
    } else {
        None
    };
    let n = match solver {
        Solver::Fzero | Solver::Fminbnd => Some(1),
        _ if nvars.is_some() => nvars,
        Solver::Linprog | Solver::Intlinprog => numel(F),
        Solver::Quadprog => get(H).size(ws).map(|(r, _)| r).or(numel(F)),
        _ => numel(X0),
    };
    // Size checks before running.
    if solver.takes_x0() && numel(X0) == Some(0) {
        return Err("Optimize: the start point x0 is empty".into());
    }
    if solver == Solver::Fzero && numel(X0).is_some_and(|k| k > 2) {
        return Err(
            "Optimize: fzero needs one start value or a bracket [a b]; for several unknowns choose System"
                .into(),
        );
    }
    if solver == Solver::Fminbnd {
        for slot in [Lb, Ub] {
            if numel(slot).is_some_and(|k| k != 1) {
                return Err(format!(
                    "Optimize: fminbnd needs a single value for {}",
                    slot.label()
                ));
            }
        }
    }
    if solver == Solver::Quadprog
        && let Some((r, c)) = get(H).size(ws)
    {
        if r != c {
            return Err(format!("Optimize: H must be square; it is {r}×{c}"));
        }
        if let Some(k) = numel(F)
            && k != r
        {
            return Err(format!("Optimize: f has {k} values but H is {r}×{r}"));
        }
    }
    if let Some(n) = n {
        let owner = if solver.takes_x0() && solver != Solver::Fzero {
            "x0 has"
        } else {
            "the problem has"
        };
        for slot in [Lb, Ub] {
            if let Some(k) = numel(slot)
                && k != n
            {
                return Err(format!(
                    "Optimize: {} has {k} values but {owner} {n} variables",
                    slot.label()
                ));
            }
        }
        for (lhs, rhs) in [(A, B), (Aeq, Beq)] {
            let Some((rows, cols)) = get(lhs).size(ws) else {
                continue;
            };
            if cols != n {
                return Err(format!(
                    "Optimize: {} has {cols} columns but {owner} {n} variables",
                    lhs.label()
                ));
            }
            if let Some(k) = numel(rhs)
                && k != rows
            {
                return Err(format!(
                    "Optimize: {} has {k} values but {} has {rows} rows",
                    rhs.label(),
                    lhs.label()
                ));
            }
        }
        if let Data::Literal { values, .. } = get(Intcon)
            && values
                .iter()
                .any(|v| v.fract() != 0. || *v < 1. || *v > n as f64)
        {
            return Err(format!(
                "Optimize: integer variables must be indices from 1 to {n}"
            ));
        }
    }
    // The objective and the nonlinear constraints.
    let parameters: &[&str] = if problem == Problem::CurveFit {
        &["x", "xdata"]
    } else {
        &["x"]
    };
    let objective = if problem.has_function() {
        let what = match problem {
            Problem::Equations => "equations F(x)",
            Problem::LeastSquares => "residual function",
            Problem::CurveFit => "model",
            _ => "objective",
        };
        Some(function(
            &spec.objective,
            what,
            parameters,
            &mut inputs,
            ws,
        )?)
    } else {
        None
    };
    let nonlinear = if constraints.nonlinear {
        let mut f = spec.nonlcon.clone();
        let ceq = check_text(&spec.ceq, "ceq(x)")?;
        if f.source == Source::Expression && f.expression.trim().is_empty() {
            if ceq.is_empty() {
                return Err("Optimize: type c(x) or ceq(x), or turn Nonlinear off".into());
            }
            f.expression = "[]".into();
        }
        let source = function(&f, "nonlinear constraint c(x)", &["x"], &mut inputs, ws)?;
        Some(match source {
            Callable::Typed(c) => {
                read_inputs(&mut inputs, &ceq, &["x"], ws);
                let ceq = if ceq.is_empty() { "[]".to_owned() } else { ceq };
                Callable::Typed(format!("deal({c}, {ceq})"))
            }
            other => other,
        })
    } else {
        None
    };
    // Names: outputs first, then the definitions in order.
    let names = app.output_names(ws, &inputs)?;
    let mut namer = Namer {
        ws,
        names: &names,
        definitions: Vec::new(),
        defined: Vec::new(),
    };
    let fun = match objective {
        Some(Callable::Named(reference)) => Some(reference),
        Some(Callable::Typed(body)) => {
            Some(namer.define("fun", format!("@({}) {body}", parameters.join(", "))))
        }
        None => None,
    };
    let mut slots: [Option<String>; Slot::COUNT] = Default::default();
    for (slot, value) in &data {
        slots[slot.index()] = match value {
            Data::Empty => None,
            Data::Variable(name) => Some(name.clone()),
            Data::Literal { rows, cols, values } => {
                Some(namer.define(slot.base(), matrix_text(*rows, *cols, values)))
            }
            Data::Expression(text) => Some(namer.define(slot.base(), text.clone())),
        };
    }
    let nonlcon = match nonlinear {
        Some(Callable::Named(reference)) => Some(reference),
        Some(Callable::Typed(body)) => Some(namer.define("nonlcon", format!("@(x) {body}"))),
        None => None,
    };
    let pairs = changed_options(solver, &spec.options, n)?;
    let options = (!pairs.is_empty()).then(|| {
        let settings: Vec<String> = pairs
            .iter()
            .map(|(key, value)| format!("'{key}', {value}"))
            .collect();
        let name = namer.define(
            "options",
            format!("optimoptions('{}', {})", solver.name(), settings.join(", ")),
        );
        // The options line follows the data, so it is kept apart.
        let line = namer.definitions.pop().unwrap_or_default();
        (name, line)
    });
    let seed = if solver.stochastic() {
        match spec.seed.trim().parse::<u32>() {
            Ok(seed) => Some(seed.to_string()),
            Err(_) => {
                return Err(
                    "Optimize: the random seed must be a whole number from 0 to 4294967295".into(),
                );
            }
        }
    } else {
        None
    };
    let tolerance = if solver.options().contains(&Opt::ConstraintTolerance) {
        parse_option(
            Opt::ConstraintTolerance,
            spec.options.value(Opt::ConstraintTolerance),
        )?
        .unwrap_or(1e-6)
    } else {
        1e-6
    };
    let Namer {
        definitions,
        defined,
        ..
    } = namer;
    Ok(Plan {
        solver,
        problem,
        constraints,
        definitions,
        defined,
        inputs,
        fun,
        slots,
        nonlcon,
        nvars,
        options,
        seed,
        names,
        tolerance,
    })
}

/// A function as the script refers to it.
enum Callable {
    /// A handle variable or `@file`.
    Named(String),
    /// The body of an anonymous function.
    Typed(String),
}

fn add_input(inputs: &mut Vec<String>, name: &str) {
    if !inputs.iter().any(|n| n == name) {
        inputs.push(name.to_owned());
    }
}
/// Workspace variables a typed expression reads (its parameters aside).
fn read_inputs(inputs: &mut Vec<String>, text: &str, parameters: &[&str], ws: &Snapshot) {
    for word in identifiers(text) {
        if ws.contains(&word) && !parameters.contains(&word.as_str()) {
            add_input(inputs, &word);
        }
    }
}

/// The objective or the constraint function from its source.
fn function(
    f: &Function,
    what: &str,
    parameters: &[&str],
    inputs: &mut Vec<String>,
    ws: &Snapshot,
) -> Result<Callable, String> {
    match f.source {
        Source::Expression => {
            let body = check_text(&f.expression, what)?;
            if body.is_empty() {
                return Err(format!("Optimize: type the {what}"));
            }
            let assignment = format!("app_function = @({}) {body};", parameters.join(", "));
            let statements = matrust::syntax::parse(&assignment)
                .map_err(|e| format!("Optimize: {what}: use exactly one expression: {e}"))?;
            if !matches!(
                statements.as_slice(),
                [matrust::syntax::Statement {
                    kind: matrust::syntax::StatementKind::Evaluate(matrust::syntax::Evaluation {
                        expr: matrust::syntax::Expr::Anonymous(_),
                        ..
                    }),
                    ..
                }]
            ) {
                return Err(format!("Optimize: {what}: use exactly one expression"));
            }
            read_inputs(inputs, &body, parameters, ws);
            Ok(Callable::Typed(body))
        }
        Source::Workspace => {
            let name = f.variable.trim();
            if name.is_empty() {
                return Err(format!(
                    "Optimize: choose a workspace function handle for the {what}"
                ));
            }
            match ws.get(name) {
                Some(Array::Handle(_)) => {
                    add_input(inputs, name);
                    Ok(Callable::Named(name.to_owned()))
                }
                Some(_) => Err(format!(
                    "Optimize: workspace variable {name} is not a function handle"
                )),
                None => Err(format!(
                    "Optimize: workspace variable {name} does not exist"
                )),
            }
        }
        Source::File => {
            let name = f.file.trim();
            let name = name.strip_suffix(".m").unwrap_or(name);
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Optimize: type the {what}'s function file name, for example myfun"
                ));
            }
            Ok(Callable::Named(format!("@{name}")))
        }
    }
}

/// Unused names for the script's definitions.
struct Namer<'a> {
    ws: &'a Snapshot,
    names: &'a [String; 4],
    definitions: Vec<String>,
    defined: Vec<String>,
}
impl Namer<'_> {
    fn define(&mut self, base: &str, value: String) -> String {
        let name = unused_name(base, |n| {
            self.ws.contains(n)
                || self.names.iter().any(|m| m == n)
                || self.defined.iter().any(|d| d == n)
        });
        self.definitions.push(format!("{name} = {value};"));
        self.defined.push(name.clone());
        name
    }
}

/// The script: definitions, options, the solver call and display lines.
fn script_text(plan: &Plan) -> String {
    let mut script = format!("% {} with {}\n", plan.problem.label(), plan.solver.name());
    for line in &plan.definitions {
        script.push_str(line);
        script.push('\n');
    }
    if let Some((_, line)) = &plan.options {
        script.push_str(line);
        script.push('\n');
    }
    if let Some(seed) = &plan.seed {
        script.push_str(&format!("rng({seed});\n"));
    }
    script.push_str(&format!(
        "{} = {};\n{}\n{}\n",
        plan.outputs(),
        plan.call(plan.fun.as_deref()),
        plan.names[0],
        plan.names[1]
    ));
    script
}

/// Private lines after the script: the results, a recorded second solve,
/// the map and the fit; then the helper functions.
fn probe_text(plan: &Plan, run: u64) -> String {
    use Slot::*;
    let [x, value, flag, output] = &plan.names;
    let solver = plan.solver;
    let or_empty = |slot: Slot| plan.slot(slot).unwrap_or("[]").to_owned();
    let mut code = format!(
        "% run {run}\n\
         app_x = double({x}); app_shape = size(app_x); app_x = app_x(:);\n\
         app_fval = double({value}); app_fval = app_fval(:);\n\
         app_flag = double({flag});\n\
         app_iter = NaN; app_fc = NaN; app_msg = ''; app_cv = NaN;\n\
         app_tol = {};\n",
        literal(plan.tolerance)
    );
    if !solver.plain() {
        code.push_str(&format!(
            "app_o = {output};\n\
             if isfield(app_o, 'iterations'), app_iter = double(app_o.iterations); \
             elseif isfield(app_o, 'generations'), app_iter = double(app_o.generations); end\n\
             if isfield(app_o, 'funcCount'), app_fc = double(app_o.funcCount); \
             elseif isfield(app_o, 'funccount'), app_fc = double(app_o.funccount); end\n\
             if isfield(app_o, 'message'), app_msg = char(app_o.message); end\n\
             if isfield(app_o, 'constrviolation'), app_cv = double(app_o.constrviolation); \
             elseif isfield(app_o, 'maxconstraint'), app_cv = double(app_o.maxconstraint); end\n"
        ));
    }
    code.push_str(&match plan.slot(X0) {
        Some(x0) if solver.takes_x0() => format!("app_x0 = double({x0}); app_x0 = app_x0(:);\n"),
        _ => "app_x0 = [];\n".into(),
    });
    // Constraint violation of a point.
    let constrained = plan.constraints.any();
    code.push_str(&format!(
        "app_viol = @(p) app_violation(p, {}, {}, {}, {}, {}, {}, {}, app_shape);\n",
        or_empty(Lb),
        or_empty(Ub),
        or_empty(A),
        or_empty(B),
        or_empty(Aeq),
        or_empty(Beq),
        plan.nonlcon.as_deref().unwrap_or("[]"),
    ));
    // The value the map draws and the recorder logs.
    let mut lambda = String::new();
    if let Some(fun) = &plan.fun {
        code.push_str(&format!("app_fun = {fun};\n"));
        let (merit, record) = match solver {
            Solver::Fsolve | Solver::Lsqnonlin => (
                "app_merit = @(p) app_sq(app_fun(p));\n".to_owned(),
                "app_rf = @(p) app_record(p, app_fun, app_book, 1);\n".to_owned(),
            ),
            Solver::Lsqcurvefit => {
                let (xdata, ydata) = (or_empty(Xdata), or_empty(Ydata));
                (
                    format!("app_merit = @(p) app_sq(app_fun(p, {xdata}) - {ydata});\n"),
                    format!(
                        "app_rf = @(p, app_q) app_record_fit(p, app_q, app_fun, {ydata}, app_book);\n"
                    ),
                )
            }
            _ => (
                "app_merit = app_fun;\n".to_owned(),
                "app_rf = @(p) app_record(p, app_fun, app_book, 0);\n".to_owned(),
            ),
        };
        code.push_str(&merit);
        code.push_str("app_book = containers.Map('KeyType', 'double', 'ValueType', 'any');\n");
        code.push_str(&record);
        if let Some(seed) = &plan.seed {
            code.push_str(&format!("rng({seed});\n"));
        }
        code.push_str("app_t0 = tic;\n");
        if solver.has_lambda() {
            code.push_str(&format!(
                "[~, ~, ~, ~, app_lambda] = {};\n",
                plan.call(Some("app_rf"))
            ));
            lambda = "app_lambda".into();
        } else {
            code.push_str(&format!("app_rerun = {};\n", plan.call(Some("app_rf"))));
        }
        // Seconds per evaluation: the map's grid fits a time budget.
        code.push_str(
            "app_each = toc(app_t0) / max(1, double(app_book.Count));\n\
             if app_book.Count > 0\n\
             app_E = cell2mat(values(app_book)');\n\
             else\n\
             app_E = zeros(0, numel(app_x) + 1);\n\
             end\n",
        );
    } else {
        // Linear and quadratic programs: f'*x (plus 0.5*x'*H*x).
        let f = or_empty(F);
        code.push_str(&format!(
            "app_fv = double({f}); app_fv = app_fv(:);\n\
             if isempty(app_fv), app_fv = zeros(numel(app_x), 1); end\n"
        ));
        code.push_str(&if solver == Solver::Quadprog {
            format!(
                "app_H = double({});\napp_merit = @(p) 0.5 * p(:)' * app_H * p(:) + app_fv' * p(:);\n",
                or_empty(H)
            )
        } else {
            "app_merit = @(p) app_fv' * p(:);\n".into()
        });
        if solver.has_lambda() {
            code.push_str(&format!(
                "[~, ~, ~, ~, app_lambda] = {};\n",
                plan.call(None)
            ));
            lambda = "app_lambda".into();
        }
        code.push_str("app_E = zeros(0, numel(app_x) + 1);\napp_each = 0;\n");
    }
    // Lagrange multipliers: one column per field, [count; values; padding].
    code.push_str(&format!(
        "app_lnames = {{{}}};\napp_lcols = cell(1, 6);\n",
        LAMBDA
            .iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    code.push_str("for app_k = 1:6\napp_lcols{app_k} = [];\nend\n");
    if !lambda.is_empty() {
        code.push_str(&format!(
            "for app_k = 1:6\n\
             if isfield({lambda}, app_lnames{{app_k}})\n\
             app_lcols{{app_k}} = double({lambda}.(app_lnames{{app_k}}));\n\
             app_lcols{{app_k}} = app_lcols{{app_k}}(:);\n\
             end\n\
             end\n"
        ));
    }
    code.push_str(
        "app_lrows = 1;\n\
         for app_k = 1:6\napp_lrows = max(app_lrows, numel(app_lcols{app_k}) + 1);\nend\n\
         app_lambda_all = zeros(app_lrows, 6);\n\
         for app_k = 1:6\n\
         app_c = app_lcols{app_k};\n\
         app_lambda_all(1, app_k) = numel(app_c);\n\
         app_lambda_all(2:numel(app_c) + 1, app_k) = app_c;\n\
         end\n",
    );
    // The best point after each evaluation (feasibility first when constrained).
    code.push_str("app_n = size(app_E, 1);\n");
    if constrained {
        code.push_str(
            "app_V = zeros(app_n, 1);\n\
             for app_k = 1:app_n\n\
             try\napp_V(app_k) = app_viol(app_E(app_k, 1:end-1));\ncatch\napp_V(app_k) = NaN;\nend\n\
             end\n",
        );
    } else {
        code.push_str("app_V = zeros(0, 1);\n");
    }
    let absolute = if solver == Solver::Fzero {
        "app_f = abs(app_f);\n"
    } else {
        ""
    };
    code.push_str(&format!(
        "app_keep = false(app_n, 1); app_bf = NaN; app_bv = Inf;\n\
         for app_k = 1:app_n\n\
         app_f = app_E(app_k, end);\n\
         {absolute}\
         if isnan(app_f), continue; end\n\
         app_v = 0;\n\
         if numel(app_V) == app_n, app_v = app_V(app_k); end\n\
         if isnan(app_v), continue; end\n\
         if app_v <= app_tol\n\
         app_better = app_bv > app_tol || isnan(app_bf) || app_f < app_bf;\n\
         else\n\
         app_better = app_bv > app_tol && app_v < app_bv;\n\
         end\n\
         if app_better, app_keep(app_k) = true; app_bf = app_f; app_bv = app_v; end\n\
         end\n"
    ));
    // The map: one or two variables, over the start, the solution and the path.
    let finite_bounds = (solver.linear() || matches!(plan.problem, Problem::Global))
        && plan.slot(Lb).is_some()
        && plan.slot(Ub).is_some();
    code.push_str(&format!(
        "app_gx = []; app_gy = []; app_gz = []; app_gv = [];\n\
         app_nv = numel(app_x);\n\
         if app_nv == 1 || app_nv == 2\n\
         app_P = [reshape(app_x0, [], app_nv); app_x'; app_E(app_keep, 1:app_nv)];\n\
         app_P = app_P(all(isfinite(app_P), 2), :);\n\
         if isempty(app_P), app_P = zeros(1, app_nv); end\n\
         app_lo = min(app_P, [], 1); app_hi = max(app_P, [], 1);\n\
         app_half = 1.25 * (app_hi - app_lo) / 2 + 0.1 * max(abs([app_lo; app_hi]), [], 1);\n\
         app_half(app_half == 0) = 1;\n\
         {}\
         app_mid = (app_lo + app_hi) / 2;\n\
         app_lo = app_mid - app_half; app_hi = app_mid + app_half;\n\
         {}\
         if app_nv == 2\n\
         app_m = min({GRID}, floor(sqrt({MAP_SECONDS} / max(app_each, 1e-9))));\n\
         else\n\
         app_m = min({LINE_POINTS}, floor({MAP_SECONDS} / max(app_each, 1e-9)));\n\
         end\n\
         if app_m < {MIN_GRID}, app_nv = 0; end\n\
         end\n\
         if app_nv == 1 || app_nv == 2\n\
         app_gx = linspace(app_lo(1), app_hi(1), app_m);\n\
         if app_nv == 2, app_gy = linspace(app_lo(2), app_hi(2), app_m); end\n\
         app_gz = nan(max(1, numel(app_gy)), numel(app_gx)); app_gv = zeros(size(app_gz));\n\
         for app_i = 1:size(app_gz, 1)\n\
         for app_j = 1:numel(app_gx)\n\
         if app_nv == 2, app_p = [app_gx(app_j); app_gy(app_i)]; else, app_p = app_gx(app_j); end\n\
         app_p = reshape(app_p, app_shape);\n\
         try\napp_gz(app_i, app_j) = real(double(app_merit(app_p)));\ncatch\nend\n",
        if solver.linear() {
            // No start point or path: show some room around the solution.
            "app_half = max(app_half, max(1, 0.5 * max(abs(app_x))));\n"
        } else {
            ""
        },
        if finite_bounds {
            format!(
                "app_bl = double({}); app_bu = double({});\n\
                 if numel(app_bl) == app_nv && numel(app_bu) == app_nv && all(isfinite([app_bl(:); app_bu(:)]))\n\
                 app_pad = 0.05 * (app_bu(:)' - app_bl(:)');\n\
                 app_lo = min(app_lo, app_bl(:)' - app_pad); app_hi = max(app_hi, app_bu(:)' + app_pad);\n\
                 end\n",
                or_empty(Lb),
                or_empty(Ub)
            )
        } else {
            String::new()
        },
    ));
    if constrained {
        code.push_str("try\napp_gv(app_i, app_j) = app_viol(app_p);\ncatch\napp_gv(app_i, app_j) = NaN;\nend\n");
    }
    code.push_str("end\nend\nend\n");
    // Curve fitting: the data and the model over its range.
    code.push_str("app_xd = []; app_yd = []; app_xq = []; app_yq = [];\n");
    if solver == Solver::Lsqcurvefit {
        let (xdata, ydata) = (or_empty(Xdata), or_empty(Ydata));
        code.push_str(&format!(
            "if isvector({xdata}) && numel({xdata}) == numel({ydata})\n\
             app_xd = double({xdata}(:)); app_yd = double({ydata}(:));\n\
             try\n\
             app_q = linspace(min(app_xd), max(app_xd), {LINE_POINTS});\n\
             if ~isrow({xdata}), app_q = app_q'; end\n\
             app_yq = double(app_fun({x}, app_q)); app_yq = app_yq(:); app_xq = app_q(:);\n\
             if numel(app_yq) ~= numel(app_xq), app_xq = []; app_yq = []; end\n\
             catch\n\
             app_xq = []; app_yq = [];\n\
             end\n\
             end\n"
        ));
    }
    code.push_str(HELPERS);
    code
}

/// Grid points per side of a two-variable map.
pub const GRID: usize = 41;
/// Points of a one-variable map and of the fitted curve.
pub const LINE_POINTS: usize = 201;
/// Seconds the map's grid may take, from the time of one evaluation in the
/// recorded solve; below `MIN_GRID` points a side there is no map.
const MAP_SECONDS: f64 = 3.;
const MIN_GRID: usize = 11;

/// Local functions of the probe: the recorder (it keeps the first 10000
/// evaluations), the sum of squares and the constraint violation.
const HELPERS: &str = "function F = app_record(p, fun, book, squares)\n\
F = fun(p);\n\
if book.Count < 10000\n\
if squares, v = app_sq(F); else, v = real(double(F)); end\n\
book(book.Count + 1) = [double(p(:))', v];\n\
end\n\
end\n\
function F = app_record_fit(p, q, fun, target, book)\n\
F = fun(p, q);\n\
if book.Count < 10000\n\
book(book.Count + 1) = [double(p(:))', app_sq(F - target)];\n\
end\n\
end\n\
function s = app_sq(F)\n\
s = sum(abs(double(F(:))).^2);\n\
end\n\
function v = app_violation(p, lb, ub, A, b, Aeq, beq, nonlcon, shape)\n\
p = double(p(:)); v = 0;\n\
if ~isempty(lb), v = max([v; double(lb(:)) - p]); end\n\
if ~isempty(ub), v = max([v; p - double(ub(:))]); end\n\
if ~isempty(A), v = max([v; double(A) * p - double(b(:))]); end\n\
if ~isempty(Aeq), v = max([v; abs(double(Aeq) * p - double(beq(:)))]); end\n\
if ~isempty(nonlcon)\n\
[c, ceq] = nonlcon(reshape(p, shape));\n\
v = max([v; double(c(:)); abs(double(ceq(:)))]);\n\
end\n\
end\n";

impl App for Optimize {
    fn kind(&self) -> AppKind {
        AppKind::Optimize
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        Ok(script_text(&self.plan(ws)?))
    }
    fn inputs(&self) -> Vec<String> {
        self.inputs.clone()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        match self.plan(ws) {
            Ok(plan) => (
                probe_text(&plan, self.runs),
                PROBE.iter().map(|s| (*s).to_owned()).collect(),
            ),
            Err(_) => (String::new(), Vec::new()),
        }
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let plan = self.plan(ws).ok();
        self.planned = plan.as_ref().map(|p| (p.solver, p.problem, p.constraints));
        self.inputs = plan.map(|p| p.inputs).unwrap_or_default();
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let Some(planned) = self.planned else {
            return Err("Optimize: the run finished after Stop".into());
        };
        let solution = Solution::from_outputs(planned, outputs)?;
        if solution.map.is_none() && self.view == View::Map {
            self.view = View::Progress;
        }
        if solution.fit.is_none() && self.view == View::Fit {
            self.view = View::Progress;
        }
        self.result = Some(solution);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.run_ui(ui);
        section(ui);
        problem_ui(ui, &mut self.spec, ws);
        section(ui);
        let solver = self.spec.solver(ws);
        data_ui(ui, &mut self.spec, solver, ws);
        section(ui);
        options_ui(ui, &mut self.spec, solver, ws);
    }
    fn main_caption(&self) -> &'static str {
        "SOLVER"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let views: Vec<View> = [View::Progress, View::Map, View::Fit]
            .into_iter()
            .filter(|v| match v {
                View::Progress => true,
                View::Map => self.result.as_ref().is_some_and(|r| r.map.is_some()),
                View::Fit => self.result.as_ref().is_some_and(|r| r.fit.is_some()),
            })
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in &views {
                let tab = crate::tokens::tab(ui, self.view == *view, view.label());
                if *view == View::Progress {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::Optimize),
                    );
                }
                if tab.clicked() {
                    self.view = *view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(result) = &self.result else {
            crate::tokens::helper(
                ui,
                "No result yet: set up the problem and press Run. The status line says what is missing.",
            );
            return;
        };
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        match self.view {
            View::Progress => progress_ui(ui, result, &mut self.log_scale, height),
            View::Map => {
                if let Some(map) = &result.map {
                    map_ui(ui, result, map, height);
                }
            }
            View::Fit => {
                if let Some(fit) = &result.fit {
                    fit_plot(ui, fit, height);
                }
            }
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(r) = &self.result else {
            return ["Solver", "x", "Objective", "Exit flag"]
                .into_iter()
                .map(|label| (label, "—".to_owned()))
                .collect();
        };
        let count = |v: Option<f64>| v.map_or("—".to_owned(), |v| format!("{}", v as i64));
        let mut rows = vec![
            ("Solver", r.solver.name().to_owned()),
            ("x", vector_text(&r.x)),
            r.value_row(),
            ("Exit flag", format!("{}", r.exitflag as i64)),
            (
                if r.solver == Solver::Ga {
                    "Generations"
                } else {
                    "Iterations"
                },
                count(r.iterations),
            ),
            ("Function evaluations", count(r.evaluations)),
        ];
        if r.constrained
            && let Some(v) = r.violation
        {
            rows.push(("Constraint violation", short(v)));
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        if let Some(r) = &self.result {
            let ink = if r.exitflag > 0. {
                p.success
            } else if r.exitflag == 0. {
                p.warning
            } else {
                p.error
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(exit_meaning(r.solver, r.exitflag)).color(ink),
                )
                .wrap(),
            );
            if !r.message.is_empty() {
                ui.add_space(crate::tokens::XS);
                ui.add(
                    egui::Label::new(egui::RichText::new(&r.message).color(p.text_muted)).wrap(),
                );
            }
            if !r.lambda.is_empty() {
                ui.add_space(crate::tokens::S);
                ui.label(egui::RichText::new("Lagrange multipliers").color(p.text_muted));
                egui::Grid::new("optimize_lambda")
                    .num_columns(2)
                    .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                    .show(ui, |ui| {
                        for (name, vals) in &r.lambda {
                            ui.label(egui::RichText::new(*name).color(p.text_muted));
                            ui.label(egui::RichText::new(vector_text(vals)).color(p.text));
                            ui.end_row();
                        }
                    });
            }
            ui.add_space(crate::tokens::M);
        }
        let plain = self
            .applied
            .as_ref()
            .unwrap_or(&self.spec)
            .solver(ws)
            .plain();
        let defaults = self.names_in_use(ws);
        for (k, (_, label)) in NAMES.iter().enumerate() {
            if k == 3 && plain {
                continue;
            }
            if !self.edited[k] {
                self.names[k] = defaults[k].clone();
            }
            let before = self.names[k].clone();
            name_field(ui, label, &mut self.names[k], ws);
            self.edited[k] |= self.names[k] != before;
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        self.plan(ws).map(|p| p.exported()).unwrap_or_default()
    }
    fn summary(&self) -> Option<String> {
        let r = self.result.as_ref()?;
        let (label, value) = r.value_row();
        let mut text = format!(
            "{} · {label} {value} · exit flag {}",
            r.solver.name(),
            r.exitflag as i64
        );
        if let Some(k) = r.iterations {
            text.push_str(&format!(" · {} iterations", k as i64));
        }
        Some(text)
    }
}

impl Optimize {
    fn run_ui(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        ui.horizontal(|ui| {
            let width = (ui.available_width() - crate::tokens::S) / 2.;
            let size = egui::vec2(width, crate::tokens::CONTROL);
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("Run").color(p.on_accent))
                        .fill(p.accent_fill)
                        .min_size(size),
                )
                .on_hover_text("Solve with the current inputs")
                .clicked()
            {
                self.run();
            }
            if ui
                .add_enabled(
                    self.running && self.applied.is_some(),
                    egui::Button::new("Stop").min_size(size),
                )
                .on_hover_text("Cancel the running solve")
                .clicked()
            {
                self.stop();
            }
        });
        let note = if self.running && self.applied.is_some() {
            Some("Solving… Stop cancels.")
        } else if self.stopped {
            Some("Stopped. Press Run to solve again.")
        } else if self.applied.is_none() {
            Some("Press Run to solve.")
        } else if self.applied.as_ref() != Some(&self.spec) {
            Some("The inputs changed since the last run.")
        } else {
            None
        };
        if let Some(note) = note {
            crate::tokens::helper(ui, note);
        }
    }
}

// ---------------------------------------------------------------------------
// Inputs

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    choices: &[(T, String)],
    shown: String,
) {
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (choice, label) in choices {
                ui.selectable_value(value, *choice, label.as_str());
            }
        });
}

fn problem_ui(ui: &mut egui::Ui, spec: &mut Spec, ws: &Snapshot) {
    field(ui, "Problem", |ui| {
        let choices: Vec<(Problem, String)> = Problem::ALL
            .iter()
            .map(|p| (*p, p.label().to_owned()))
            .collect();
        let shown = spec.problem.label().to_owned();
        combo(ui, "optimize_problem", &mut spec.problem, &choices, shown);
    });
    if spec.problem == Problem::Equations {
        field(ui, "Unknowns", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, !spec.system, "One (scalar root)").clicked() {
                    spec.system = false;
                }
                if crate::tokens::toggle(ui, spec.system, "System").clicked() {
                    spec.system = true;
                }
            });
        });
    }
    let allowed: Vec<Constraint> = Constraint::ALL
        .into_iter()
        .filter(|k| spec.problem.allows(*k))
        .collect();
    if !allowed.is_empty() {
        field(ui, "Constraints", |ui| {
            for kind in allowed {
                let mut on = spec.constraints.get(kind);
                if ui.checkbox(&mut on, kind.label()).changed() {
                    spec.constraints.set(kind, on);
                }
            }
        });
    }
    let valid = spec.solvers(ws);
    let current = spec.solver(ws);
    field(ui, "Solver", |ui| {
        let mut chosen = current;
        let choices: Vec<(Solver, String)> = valid
            .iter()
            .enumerate()
            .map(|(k, s)| {
                (
                    *s,
                    if k == 0 {
                        format!("{} (recommended)", s.name())
                    } else {
                        s.name().to_owned()
                    },
                )
            })
            .collect();
        let shown = choices
            .iter()
            .find(|(s, _)| *s == current)
            .map_or(current.name().to_owned(), |(_, l)| l.clone());
        combo(ui, "optimize_solver", &mut chosen, &choices, shown);
        if chosen != current {
            // The recommended one follows later changes of the problem.
            spec.solver = (chosen != valid[0]).then_some(chosen);
        }
    });
}

/// Real numeric workspace variables: name and size text.
fn numeric_variables(ws: &Snapshot) -> Vec<(String, String)> {
    ws.variables()
        .filter(|(_, a)| numeric(a))
        .map(|(name, a)| {
            let dims: Vec<String> = a.dims().iter().map(usize::to_string).collect();
            (name.to_owned(), dims.join("×"))
        })
        .collect()
}
fn handle_variables(ws: &Snapshot) -> Vec<String> {
    ws.variables()
        .filter(|(_, a)| matches!(a, Array::Handle(_)))
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// A typed value or a workspace variable, with a variable picker.
fn data_field(ui: &mut egui::Ui, spec: &mut Spec, slot: Slot, hint: &str, ws: &Snapshot) {
    let p = crate::tokens::palette(ui.ctx());
    let variables = numeric_variables(ws);
    field(ui, slot.label(), |ui| {
        let text = &mut spec.data[slot.index()];
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 32.).max(60.);
            ui.add(
                crate::tokens::input(egui::TextEdit::singleline(text).char_limit(4000))
                    .hint_text(hint)
                    .desired_width(width),
            );
            egui::ComboBox::from_id_salt(("optimize_pick", slot.base()))
                .width(20.)
                .selected_text("")
                .show_ui(ui, |ui| {
                    if variables.is_empty() {
                        ui.label(egui::RichText::new("No numeric variables").color(p.text_muted));
                    }
                    for (name, size) in &variables {
                        if ui
                            .selectable_label(*text == *name, format!("{name}  {size}"))
                            .clicked()
                        {
                            *text = name.clone();
                        }
                    }
                })
                .response
                .on_hover_text("Use a workspace variable");
        });
        let resolved = Data::resolve(text, ws);
        if let Data::Expression(expression) = &resolved {
            super::expression_message(ui, check_text(expression, slot.label()).map(|_| ()));
        }
        let note = match (&resolved, resolved.size(ws)) {
            (Data::Variable(name), Some((r, c))) => {
                Some(format!("Workspace variable {name}, {r}×{c}"))
            }
            (Data::Literal { .. }, Some((r, c))) => Some(format!("Typed, {r}×{c}")),
            _ => None,
        };
        if let Some(note) = note {
            crate::tokens::helper(ui, note);
        }
    });
}

/// Expression, workspace handle or function file.
fn function_ui(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    f: &mut Function,
    expression_label: &str,
    help: &str,
    ws: &Snapshot,
) {
    field(ui, label, |ui| {
        ui.horizontal_wrapped(|ui| {
            for source in Source::ALL {
                if crate::tokens::toggle(ui, f.source == source, source.label()).clicked() {
                    f.source = source;
                }
            }
        });
    });
    let width = ui.available_width();
    match f.source {
        Source::Expression => {
            field(ui, expression_label, |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut f.expression).char_limit(4000),
                    )
                    .desired_width(width),
                );
                super::expression_message(
                    ui,
                    check_text(&f.expression, expression_label).map(|_| ()),
                );
                crate::tokens::helper(ui, help);
            });
        }
        Source::Workspace => {
            let handles = handle_variables(ws);
            if f.variable.is_empty()
                && let Some(first) = handles.first()
            {
                f.variable = first.clone();
            }
            field(ui, "Function handle", |ui| {
                let shown = if f.variable.is_empty() {
                    "Choose a variable".to_owned()
                } else {
                    f.variable.clone()
                };
                egui::ComboBox::from_id_salt((id, "handle"))
                    .width(ui.available_width())
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for name in &handles {
                            ui.selectable_value(&mut f.variable, name.clone(), name.as_str());
                        }
                    });
                if handles.is_empty() {
                    crate::tokens::helper(ui, "No function handles in the workspace");
                }
            });
        }
        Source::File => {
            field(ui, "Function name", |ui| {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(&mut f.file).char_limit(63))
                        .hint_text("myfun")
                        .desired_width(width),
                );
                crate::tokens::helper(ui, "A function file in the current folder");
            });
        }
    }
}

fn data_ui(ui: &mut egui::Ui, spec: &mut Spec, solver: Solver, ws: &Snapshot) {
    use Slot::*;
    let constraints = spec.constraints();
    match spec.problem {
        Problem::Linear | Problem::MixedInteger => {
            crate::tokens::helper(ui, "Minimize f'·x");
            data_field(ui, spec, F, "[-1 -2]", ws);
        }
        Problem::Quadratic => {
            crate::tokens::helper(ui, "Minimize ½·x'·H·x + f'·x");
            data_field(ui, spec, H, "[2 0; 0 2]", ws);
            data_field(ui, spec, F, "[-2 -5]", ws);
        }
        problem => {
            let (label, expression, help) = match problem {
                Problem::Equations => (
                    "Equations",
                    "F(x) = 0 where F(x) =",
                    "Use x(1), x(2), …; a column of equations for a system",
                ),
                Problem::LeastSquares => (
                    "Residuals",
                    "Minimize sum of squares of r(x) =",
                    "A vector of residuals of x(1), x(2), …",
                ),
                Problem::CurveFit => (
                    "Model",
                    "y = model(x, xdata) =",
                    "x(1), x(2), … are the coefficients; use .* and .^ with xdata",
                ),
                _ => ("Objective", "f(x) =", "Use x(1), x(2), … for the variables"),
            };
            function_ui(
                ui,
                "optimize_objective",
                label,
                &mut spec.objective,
                expression,
                help,
                ws,
            );
        }
    }
    if solver.takes_nvars() {
        field(ui, "Number of variables", |ui| {
            ui.add(
                crate::tokens::input(egui::TextEdit::singleline(&mut spec.nvars).char_limit(8))
                    .hint_text("from x0")
                    .desired_width(ui.available_width()),
            );
        });
        if spec.nvars.trim().is_empty() {
            data_field(ui, spec, X0, "[0 0]", ws);
            crate::tokens::helper(
                ui,
                "Only the size of x0 is used: the search starts from random points.",
            );
        }
    } else if solver.takes_x0() {
        let hint = if solver == Solver::Fzero {
            "1  or a bracket [0 2]"
        } else {
            "[0 0]"
        };
        data_field(ui, spec, X0, hint, ws);
    } else if solver == Solver::Fminbnd {
        // The start point's size is what offers fminbnd; keep it editable.
        data_field(ui, spec, X0, "0.5", ws);
        crate::tokens::helper(ui, "fminbnd searches between lb and ub; x0 is not used.");
    }
    if spec.problem == Problem::MixedInteger {
        data_field(ui, spec, Intcon, "[1 2]", ws);
    }
    if spec.problem == Problem::CurveFit {
        data_field(ui, spec, Xdata, "0:0.5:3", ws);
        data_field(ui, spec, Ydata, "[2 1.4 1 0.7 0.5 0.3 0.25]", ws);
    }
    if constraints.bounds {
        data_field(ui, spec, Lb, "[-Inf 0]", ws);
        data_field(ui, spec, Ub, "[Inf 10]", ws);
    }
    if constraints.inequalities {
        data_field(ui, spec, A, "[1 1]", ws);
        data_field(ui, spec, B, "1", ws);
    }
    if constraints.equalities {
        data_field(ui, spec, Aeq, "[1 -1]", ws);
        data_field(ui, spec, Beq, "0", ws);
    }
    if constraints.nonlinear {
        function_ui(
            ui,
            "optimize_nonlcon",
            "Nonlinear constraints",
            &mut spec.nonlcon,
            "c(x) ≤ 0 where c(x) =",
            "Blank for none; a column for several",
            ws,
        );
        if spec.nonlcon.source == Source::Expression {
            let width = ui.available_width();
            field(ui, "ceq(x) = 0 where ceq(x) =", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut spec.ceq).char_limit(4000),
                    )
                    .desired_width(width),
                );
                super::expression_message(ui, check_text(&spec.ceq, "ceq(x)").map(|_| ()));
            });
        } else {
            crate::tokens::helper(ui, "The function returns [c, ceq].");
        }
    }
    if solver.stochastic() {
        field(ui, "Random seed", |ui| {
            ui.add(
                crate::tokens::input(egui::TextEdit::singleline(&mut spec.seed).char_limit(10))
                    .desired_width(ui.available_width()),
            );
            crate::tokens::helper(ui, "The script seeds rng so a rerun gives the same result.");
        });
    }
}

fn options_ui(ui: &mut egui::Ui, spec: &mut Spec, solver: Solver, ws: &Snapshot) {
    let n = {
        let x0 = Data::resolve(spec.text(Slot::X0), ws).numel(ws);
        match solver {
            Solver::Fzero | Solver::Fminbnd => Some(1),
            _ if solver.takes_nvars() => spec.nvars.trim().parse().ok().or(x0),
            Solver::Linprog | Solver::Intlinprog | Solver::Quadprog => {
                Data::resolve(spec.text(Slot::F), ws).numel(ws)
            }
            _ => x0,
        }
    };
    let changed = changed_options(solver, &spec.options, n).map_or(0, |c| c.len());
    let title = if changed == 0 {
        "Options".to_owned()
    } else {
        format!("Options ({changed} changed)")
    };
    egui::CollapsingHeader::new(title)
        .id_salt("optimize_options")
        .show(ui, |ui| {
            if solver.plain() {
                crate::tokens::helper(
                    ui,
                    format!(
                        "MatRust's {} takes no options: it runs with its built-in tolerances.",
                        solver.name()
                    ),
                );
                return;
            }
            crate::tokens::helper(
                ui,
                "Blank keeps the default; the script sets only changed options.",
            );
            field(ui, "Display", |ui| {
                let mut choices: Vec<(Option<DisplayLevel>, String)> =
                    vec![(None, "Default (final)".to_owned())];
                choices.extend(
                    DisplayLevel::ALL
                        .iter()
                        .map(|d| (Some(*d), d.name().to_owned())),
                );
                let shown = spec
                    .options
                    .display
                    .map_or("Default (final)".to_owned(), |d| d.name().to_owned());
                combo(
                    ui,
                    "optimize_display",
                    &mut spec.options.display,
                    &choices,
                    shown,
                );
            });
            let algorithms = solver.algorithms();
            if !algorithms.is_empty() {
                field(ui, "Algorithm", |ui| {
                    let mut choices: Vec<(Option<&'static str>, String)> =
                        vec![(None, format!("Default ({})", algorithms[0]))];
                    choices.extend(algorithms.iter().map(|a| (Some(*a), (*a).to_owned())));
                    let shown = spec
                        .options
                        .algorithm
                        .map_or(format!("Default ({})", algorithms[0]), str::to_owned);
                    combo(
                        ui,
                        "optimize_algorithm",
                        &mut spec.options.algorithm,
                        &choices,
                        shown,
                    );
                });
            }
            for opt in solver.options() {
                let mut text = spec.options.value(*opt).to_owned();
                field(ui, opt.label(), |ui| {
                    ui.add(
                        crate::tokens::input(egui::TextEdit::singleline(&mut text).char_limit(24))
                            .hint_text(default_hint(solver, *opt, n))
                            .desired_width(ui.available_width()),
                    );
                    if let Err(error) = parse_option(*opt, &text) {
                        let p = crate::tokens::palette(ui.ctx());
                        ui.label(
                            egui::RichText::new(error.trim_start_matches("Optimize: "))
                                .size(crate::tokens::CAPTION)
                                .color(p.error),
                        );
                    }
                });
                spec.options.set(*opt, &text);
            }
        });
}

// ---------------------------------------------------------------------------
// Main view

fn chart<'a>(id: &str, height: f32, (x_label, y_label): (&str, &str)) -> egui_plot::Plot<'a> {
    egui_plot::Plot::new(id.to_owned())
        .height(height)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .auto_bounds(egui::Vec2b::TRUE)
        .x_axis_label(x_label.to_owned())
        .y_axis_label(y_label.to_owned())
        .show_background(false)
}
fn color(k: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[k];
    egui::Color32::from_rgb(r, g, b)
}

fn progress_ui(ui: &mut egui::Ui, result: &Solution, log_scale: &mut bool, height: f32) {
    let progress = &result.progress;
    if progress.best.is_empty() {
        crate::tokens::helper(
            ui,
            if result.solver.linear() {
                "Linear and quadratic solvers evaluate no function, so there is no progress to draw. \
                 The map shows the solution for one or two variables."
            } else {
                "The solver recorded no finite values."
            },
        );
        return;
    }
    let what = match result.solver {
        Solver::Fzero => "|f(x)|",
        _ => result.value_name(),
    };
    let positive = progress.best.iter().all(|p| p[1] > 0.);
    ui.horizontal_wrapped(|ui| {
        crate::tokens::helper(
            ui,
            if result.constrained {
                "Best point after each function evaluation (feasible points first)."
            } else {
                "Best value after each function evaluation."
            },
        );
        ui.add_enabled(positive, egui::Checkbox::new(log_scale, "Log scale"));
    });
    if result.x.len() <= 2 && result.map.is_none() {
        crate::tokens::helper(
            ui,
            "No objective map: one evaluation takes too long to sample a grid in a few seconds.",
        );
    }
    let log = *log_scale && positive;
    let points: Vec<[f64; 2]> = progress
        .best
        .iter()
        .map(|p| [p[0], if log { p[1].log10() } else { p[1] }])
        .collect();
    let constrained = !progress.violation.is_empty();
    let top = if constrained {
        ((height - crate::tokens::S) * 0.6).max(120.)
    } else {
        height - crate::tokens::ROW
    };
    let mut plot = chart("optimize_progress", top, ("Function evaluations", what));
    if log {
        plot = plot.y_axis_formatter(|mark, _| short(10_f64.powf(mark.value)));
    }
    plot.show(ui, |plot_ui| {
        plot_ui.line(
            egui_plot::Line::new(what, points)
                .color(color(0))
                .width(1.5),
        );
    });
    if constrained {
        ui.add_space(crate::tokens::S);
        let bottom = (height - top - crate::tokens::S - crate::tokens::ROW).max(80.);
        chart(
            "optimize_violation",
            bottom,
            ("Function evaluations", "Constraint violation"),
        )
        .show(ui, |plot_ui| {
            plot_ui.line(
                egui_plot::Line::new("Constraint violation", progress.violation.clone())
                    .color(color(1))
                    .width(1.5),
            );
        });
    }
}

/// Bands of equal area: the band of each value among the finite ones.
pub const BANDS: usize = 12;
pub fn bands(values: &[f64]) -> Vec<f64> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    values
        .iter()
        .map(|v| {
            if !v.is_finite() || n == 0 {
                return f64::NAN;
            }
            let rank = sorted.partition_point(|s| s < v);
            ((rank * BANDS) / n).min(BANDS - 1) as f64
        })
        .collect()
}

/// Fixed perceptual ramp, dark (low) to light (high).
const RAMP: [[u8; 3]; 5] = [
    [68, 1, 84],
    [59, 82, 139],
    [33, 145, 140],
    [94, 201, 98],
    [253, 231, 37],
];
fn ramp(t: f64) -> egui::Color32 {
    let t = t.clamp(0., 1.) * (RAMP.len() - 1) as f64;
    let k = (t.floor() as usize).min(RAMP.len() - 2);
    let f = t - k as f64;
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    let (a, b) = (RAMP[k], RAMP[k + 1]);
    egui::Color32::from_rgb(mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]))
}

fn map_ui(ui: &mut egui::Ui, result: &Solution, map: &Map, height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    let path_ink = color(1);
    if !map.two_d() {
        crate::tokens::helper(
            ui,
            "The function over a range around the start; dots: the best points found, ring: the solution.",
        );
        let curve: Vec<[f64; 2]> = map
            .x
            .iter()
            .zip(&map.values)
            .map(|(x, y)| [*x, *y])
            .filter(|q| q[1].is_finite())
            .collect();
        let path: Vec<[f64; 2]> = result
            .progress
            .path
            .iter()
            .zip(&result.progress.path_values)
            .filter_map(|(q, v)| Some([*q.first()?, *v]))
            .filter(|q| q[0].is_finite() && q[1].is_finite())
            .collect();
        let solution = result.x.first().copied();
        let what = if result.solver == Solver::Fzero {
            "f(x)"
        } else {
            "Objective"
        };
        chart(
            "optimize_map_line",
            height - crate::tokens::ROW,
            ("x", what),
        )
        .show(ui, |plot_ui| {
            plot_ui.line(egui_plot::Line::new(what, curve).color(color(0)).width(1.5));
            if result.solver == Solver::Fzero {
                plot_ui.hline(
                    egui_plot::HLine::new("Zero", 0.)
                        .color(p.text_muted)
                        .width(1.),
                );
            }
            plot_ui.points(
                egui_plot::Points::new("Best points", path)
                    .shape(egui_plot::MarkerShape::Circle)
                    .filled(true)
                    .radius(3.)
                    .color(path_ink),
            );
            if let Some(x) = solution {
                plot_ui.vline(
                    egui_plot::VLine::new("Solution", x)
                        .color(path_ink)
                        .width(1.),
                );
            }
        });
        return;
    }
    let values = bands(&map.values);
    let (lo, hi) = map
        .values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    crate::tokens::helper(
        ui,
        if lo <= hi {
            format!(
                "{} in {BANDS} equal-area bands, dark at {} to light at {}; grey: breaks a constraint. \
                 Line: the best points found; ring: start, dot: solution.",
                result.value_name(),
                short(lo),
                short(hi)
            )
        } else {
            "The function has no finite values over this range.".to_owned()
        },
    );
    let marked: Vec<f64> = values
        .iter()
        .zip(&map.infeasible)
        .map(|(v, out)| if *out && v.is_finite() { v + 100. } else { *v })
        .collect();
    let cols = map.x.len();
    let dx = if cols > 1 { map.x[1] - map.x[0] } else { 1. };
    let dy = if map.y.len() > 1 {
        map.y[1] - map.y[0]
    } else {
        1.
    };
    let shade = p.surface_sunken;
    let mapping = move |v: f64| {
        if !v.is_finite() {
            egui::Color32::TRANSPARENT
        } else if v >= 100. {
            let c = ramp((v - 100.) / (BANDS - 1) as f64);
            c.lerp_to_gamma(shade, 0.7)
        } else {
            ramp(v / (BANDS - 1) as f64)
        }
    };
    let path: Vec<[f64; 2]> = result
        .progress
        .path
        .iter()
        .filter(|q| q.len() == 2)
        .map(|q| [q[0], q[1]])
        .collect();
    let start: Vec<[f64; 2]> = if result.start.len() == 2 {
        vec![[result.start[0], result.start[1]]]
    } else {
        Vec::new()
    };
    let solution = (result.x.len() == 2).then(|| [result.x[0], result.x[1]]);
    chart(
        "optimize_map",
        height - crate::tokens::ROW,
        ("x(1)", "x(2)"),
    )
    .show(ui, |plot_ui| {
        plot_ui.heatmap(
            egui_plot::Heatmap::new(marked, cols)
                .at(egui_plot::PlotPoint::new(
                    map.x[0] - dx / 2.,
                    map.y[0] - dy / 2.,
                ))
                .tile_size(dx as f32, dy as f32)
                .show_labels(false)
                .custom_mapping(Box::new(mapping)),
        );
        if path.len() > 1 {
            plot_ui.line(
                egui_plot::Line::new("Path", path)
                    .color(path_ink)
                    .width(1.5),
            );
        }
        if !start.is_empty() {
            plot_ui.points(
                egui_plot::Points::new("Start", start)
                    .shape(egui_plot::MarkerShape::Circle)
                    .filled(false)
                    .radius(5.)
                    .color(path_ink),
            );
        }
        if let Some(solution) = solution {
            plot_ui.points(
                egui_plot::Points::new("Solution", vec![solution])
                    .shape(egui_plot::MarkerShape::Circle)
                    .filled(true)
                    .radius(4.)
                    .color(path_ink),
            );
        }
    });
}

fn fit_plot(ui: &mut egui::Ui, fit: &FitView, height: f32) {
    let p = crate::tokens::palette(ui.ctx());
    crate::tokens::helper(ui, "Dots: the data. Line: the model at the solution.");
    chart(
        "optimize_fit",
        height - crate::tokens::ROW,
        ("xdata", "ydata"),
    )
    .show(ui, |plot_ui| {
        plot_ui.points(
            egui_plot::Points::new("Data", fit.data.clone())
                .shape(egui_plot::MarkerShape::Circle)
                .filled(true)
                .radius(2.5)
                .color(p.text),
        );
        plot_ui.line(
            egui_plot::Line::new("Model", fit.curve.clone())
                .color(color(0))
                .width(1.5),
        );
    });
}
