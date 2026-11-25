//! Solve ODE: solves an initial value problem y' = f(t, y), or F(t, y, y') = 0
//! with ode15i, with the engine's solvers. The equations are a function
//! (a file, a workspace handle or a typed anonymous function) or are typed
//! one per line in y1..yn with named parameters; one higher-order equation
//! such as x'' = -x becomes a first-order system. One function builds the
//! script: parameters, the function, `odeset` with the options that differ
//! from the defaults, the solver call and the chosen plot (a typed event
//! condition becomes a local function at the end). The private run reads
//! back the solution, the solver's counts and the wall time. Every number
//! shown comes from engine builtins.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use crate::theme::ThemedScrollArea;
use eframe::egui;
use matrust::Array;
use matrust::syntax::Expr;
use std::collections::BTreeSet;
use std::time::Instant;

/// Most typed equations (components of a typed system).
pub const MAX_EQUATIONS: usize = 64;
/// Most parameters.
pub const MAX_PARAMETERS: usize = 32;
/// Steps of a nonstiff solver past which the app suggests a stiff one.
pub const STIFF_STEPS: f64 = 5000.;
/// The private run counts the steps of a `[t, y]` solve with a second
/// solve only when the first took less than this many seconds.
pub const COUNT_LIMIT: f64 = 2.;
/// Seconds of solving with a nonstiff solver before the app suggests a
/// stiff one.
const SLOW_SECONDS: f64 = 3.;
/// Most plotted points per line; longer solutions draw every k-th point.
const MAX_POINTS: usize = 20_000;
/// Rows of the final state and the event list.
const SHOWN_ROWS: usize = 12;
/// The local function a typed event condition becomes.
pub const EVENT_FUNCTION: &str = "zeroCrossing";

// ---------------------------------------------------------------------------
// Choices

/// Where the equations come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Typed one per line in y1..yn (or one higher-order equation).
    Typed,
    /// A function file, a workspace handle or an anonymous function.
    Function,
}

/// The solvers, named as the engine's functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solver {
    Auto,
    Ode45,
    Ode23,
    Ode113,
    Ode15s,
    Ode23s,
    Ode23t,
    Ode23tb,
    Ode15i,
}
impl Solver {
    pub const ALL: [Self; 9] = [
        Self::Auto,
        Self::Ode45,
        Self::Ode23,
        Self::Ode113,
        Self::Ode15s,
        Self::Ode23s,
        Self::Ode23t,
        Self::Ode23tb,
        Self::Ode15i,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ode45 => "ode45",
            Self::Ode23 => "ode23",
            Self::Ode113 => "ode113",
            Self::Ode15s => "ode15s",
            Self::Ode23s => "ode23s",
            Self::Ode23t => "ode23t",
            Self::Ode23tb => "ode23tb",
            Self::Ode15i => "ode15i",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Ode45 => "ode45 · nonstiff, Runge-Kutta 4(5)",
            Self::Ode23 => "ode23 · nonstiff, Runge-Kutta 2(3)",
            Self::Ode113 => "ode113 · nonstiff, Adams, variable order",
            Self::Ode15s => "ode15s · stiff, NDF, variable order",
            Self::Ode23s => "ode23s · stiff, Rosenbrock",
            Self::Ode23t => "ode23t · moderately stiff, trapezoidal",
            Self::Ode23tb => "ode23tb · stiff, TR-BDF2",
            Self::Ode15i => "ode15i · implicit F(t, y, y') = 0",
        }
    }
    /// Meant for stiff problems.
    pub fn stiff(self) -> bool {
        matches!(
            self,
            Self::Ode15s | Self::Ode23s | Self::Ode23t | Self::Ode23tb | Self::Ode15i
        )
    }
    /// Takes the app's constant mass matrix.
    pub fn mass(self) -> bool {
        matches!(self, Self::Ode15s | Self::Ode23t)
    }
    /// Locates events in MatRust (ode45 and ode23 do not yet; ode15i has
    /// no events).
    pub fn events(self) -> bool {
        matches!(
            self,
            Self::Ode113 | Self::Ode15s | Self::Ode23s | Self::Ode23t | Self::Ode23tb
        )
    }
}

/// How the event value may cross zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Either,
    Rising,
    Falling,
}
impl Direction {
    pub const ALL: [Self; 3] = [Self::Either, Self::Rising, Self::Falling];
    pub fn label(self) -> &'static str {
        match self {
            Self::Either => "Either",
            Self::Rising => "Rising",
            Self::Falling => "Falling",
        }
    }
    /// The event function's direction output.
    pub fn value(self) -> i8 {
        match self {
            Self::Either => 0,
            Self::Rising => 1,
            Self::Falling => -1,
        }
    }
}

/// Where events come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Events {
    None,
    /// A typed value whose zero crossings are events.
    Condition,
    /// A function [value, isterminal, direction] = f(t, y).
    Function,
}
impl Events {
    pub const ALL: [Self; 3] = [Self::None, Self::Condition, Self::Function];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Condition => "Condition",
            Self::Function => "Function",
        }
    }
}

/// The plot the view draws and the script makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plot {
    /// Each component against t.
    Time,
    /// Two components against each other.
    Phase,
    /// Three components in 3-D.
    Phase3,
}
impl Plot {
    pub const ALL: [Self; 3] = [Self::Time, Self::Phase, Self::Phase3];
    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "Against time",
            Self::Phase => "Phase plane",
            Self::Phase3 => "3-D phase",
        }
    }
    /// Components the plot needs.
    fn needs(self) -> usize {
        match self {
            Self::Time => 1,
            Self::Phase => 2,
            Self::Phase3 => 3,
        }
    }
}

/// The variables the script leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    T,
    Y,
    Sol,
    Te,
    Ye,
    Ie,
}
impl Out {
    pub const ALL: [Self; 6] = [Self::T, Self::Y, Self::Sol, Self::Te, Self::Ye, Self::Ie];
    pub fn base(self) -> &'static str {
        match self {
            Self::T => "t",
            Self::Y => "y",
            Self::Sol => "sol",
            Self::Te => "te",
            Self::Ye => "ye",
            Self::Ie => "ie",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::T => "Times name",
            Self::Y => "Solution name",
            Self::Sol => "Structure name",
            Self::Te => "Event times name",
            Self::Ye => "Event states name",
            Self::Ie => "Event indices name",
        }
    }
    fn index(self) -> usize {
        Self::ALL.iter().position(|o| *o == self).unwrap()
    }
}

/// An output name: the user's once edited, else the default.
#[derive(Clone, Debug, Default)]
pub struct Name {
    pub text: String,
    pub edited: bool,
}

/// The inputs of the solve.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub source: Source,
    /// Typed equations, one per line.
    pub equations: String,
    /// Parameters of typed equations: name and value.
    pub parameters: Vec<(String, String)>,
    /// A function name, a workspace handle variable or `@(t, y) ...`.
    pub function: String,
    pub y0: String,
    /// ode15i's initial slopes.
    pub yp0: String,
    /// [t0 tf] or the output times.
    pub tspan: String,
    pub solver: Solver,
    /// Auto picks a stiff solver.
    pub stiff: bool,
    /// Options as typed; blank is the default.
    pub rel_tol: String,
    pub abs_tol: String,
    pub max_step: String,
    pub initial_step: String,
    pub non_negative: String,
    /// A workspace matrix; blank is the identity.
    pub mass: String,
    pub events: Events,
    pub event_value: String,
    pub direction: Direction,
    pub terminal: bool,
    /// A function name, a workspace handle variable or a handle expression.
    pub event_function: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            source: Source::Typed,
            equations: "y2\nmu*(1 - y1^2)*y2 - y1".into(),
            parameters: vec![("mu".into(), "1".into())],
            function: String::new(),
            y0: "[2 0]".into(),
            yp0: String::new(),
            tspan: "[0 20]".into(),
            solver: Solver::Auto,
            stiff: false,
            rel_tol: String::new(),
            abs_tol: String::new(),
            max_step: String::new(),
            initial_step: String::new(),
            non_negative: String::new(),
            mass: String::new(),
            events: Events::None,
            event_value: "y1".into(),
            direction: Direction::Falling,
            terminal: true,
            event_function: String::new(),
        }
    }
}
impl Spec {
    /// The solver that runs: Auto is ode45, ode15s when stiff, and ode113
    /// when events are on (MatRust's ode45 has no event location yet).
    pub fn resolved(&self) -> Solver {
        match self.solver {
            Solver::Auto if self.stiff => Solver::Ode15s,
            Solver::Auto if self.events != Events::None => Solver::Ode113,
            Solver::Auto => Solver::Ode45,
            solver => solver,
        }
    }
    pub fn implicit(&self) -> bool {
        self.resolved() == Solver::Ode15i
    }
    /// Events reach the solve (ode15i has none).
    pub fn has_events(&self) -> bool {
        self.events != Events::None && !self.implicit()
    }
    /// The mass matrix reaches the solve.
    pub fn has_mass(&self) -> bool {
        self.resolved().mass() && !self.mass.trim().is_empty()
    }
}

// ---------------------------------------------------------------------------
// Typed equations

/// One identifier of a line: byte range, the quotes right after it, and
/// whether it is a field name (`s.y1`).
#[derive(Clone, Debug)]
struct Word {
    start: usize,
    end: usize,
    primes: usize,
    field: bool,
}

struct Scan {
    words: Vec<Word>,
    /// Where the code ends: a `%` comment, else the length.
    end: usize,
    /// A top-level `=` (not part of `==`, `<=`, `>=` or `~=`).
    assign: Option<usize>,
}

/// Finds the identifiers of one line of MATLAB code, skipping numbers,
/// char and string literals (a quote after a value is a transpose).
fn scan(text: &str) -> Result<Scan, String> {
    let b = text.as_bytes();
    let n = b.len();
    let mut words = Vec::new();
    let mut assign = None;
    let mut depth = 0usize;
    let mut after_value = false;
    let mut i = 0;
    while i < n {
        let c = b[i];
        match c {
            b'%' => {
                return Ok(Scan {
                    words,
                    end: i,
                    assign,
                });
            }
            b'"' | b'\'' if c == b'"' || !after_value => {
                i += 1;
                loop {
                    if i >= n {
                        return Err("a quote is not closed".into());
                    }
                    if b[i] == c {
                        if i + 1 < n && b[i + 1] == c {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                after_value = true;
            }
            b'\'' => {
                i += 1;
                after_value = true;
            }
            b'.' if i + 1 < n && b[i + 1] == b'\'' => {
                i += 2;
                after_value = true;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let primes = b[i..].iter().take_while(|c| **c == b'\'').count();
                words.push(Word {
                    start,
                    end: i,
                    primes,
                    field: start > 0 && b[start - 1] == b'.',
                });
                after_value = true;
            }
            c if c.is_ascii_digit() || (c == b'.' && i + 1 < n && b[i + 1].is_ascii_digit()) => {
                while i < n && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i < n
                    && b[i] == b'.'
                    && !(i + 1 < n && matches!(b[i + 1], b'*' | b'/' | b'\\' | b'^' | b'\''))
                {
                    i += 1;
                    while i < n && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < n && matches!(b[i], b'e' | b'E' | b'd' | b'D') {
                    let mut j = i + 1;
                    if j < n && matches!(b[j], b'+' | b'-') {
                        j += 1;
                    }
                    if j < n && b[j].is_ascii_digit() {
                        i = j;
                        while i < n && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                if i < n
                    && matches!(b[i], b'i' | b'j')
                    && !(i + 1 < n && (b[i + 1].is_ascii_alphanumeric() || b[i + 1] == b'_'))
                {
                    i += 1;
                }
                after_value = true;
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
                after_value = false;
            }
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
                after_value = true;
            }
            b' ' | b'\t' => i += 1,
            b'=' => {
                if i + 1 < n && b[i + 1] == b'=' {
                    i += 2;
                } else {
                    let prev = if i > 0 { b[i - 1] } else { 0 };
                    if depth == 0 && !matches!(prev, b'<' | b'>' | b'~' | b'=') && assign.is_none()
                    {
                        assign = Some(i);
                    }
                    i += 1;
                }
                after_value = false;
            }
            _ => {
                i += 1;
                after_value = false;
            }
        }
    }
    Ok(Scan {
        words,
        end: n,
        assign,
    })
}

/// `text[range]` with identifiers replaced as `name` says: `Some((with,
/// primes))` replaces the word and that many quotes after it.
type Rename<'a> = dyn FnMut(&str, usize) -> Result<Option<(String, usize)>, String> + 'a;
fn rewrite(
    text: &str,
    words: &[Word],
    range: std::ops::Range<usize>,
    name: &mut Rename<'_>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut at = range.start;
    for word in words
        .iter()
        .filter(|w| !w.field && w.start >= range.start && w.end <= range.end)
    {
        if word.start < at {
            continue;
        }
        if let Some((with, primes)) = name(&text[word.start..word.end], word.primes)? {
            out.push_str(&text[at..word.start]);
            out.push_str(&with);
            at = (word.end + primes).min(range.end);
        }
    }
    out.push_str(&text[at..range.end]);
    Ok(out.trim().to_owned())
}

/// The k of `y<k>` (`prefix` = "y") or `yp<k>`, k >= 1.
fn component(word: &str, prefix: &str) -> Option<usize> {
    let digits = word.strip_prefix(prefix)?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Identifiers of `text` (not field names), in order, once each.
fn identifiers(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let Ok(scan) = scan(line) else {
            continue;
        };
        for word in scan.words.iter().filter(|w| !w.field && w.end <= scan.end) {
            let name = &line[word.start..word.end];
            if !out.iter().any(|n| n == name) {
                out.push(name.to_owned());
            }
        }
    }
    out
}

/// A typed system as the script writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct System {
    /// The function's rows in y(k) (and yp(k)) form.
    pub rows: Vec<String>,
    /// Component names: y1..yn, or x, x', ... for a higher-order equation.
    pub labels: Vec<String>,
    /// A higher-order equation's variable and order.
    pub converted: Option<(String, usize)>,
}
impl System {
    /// The anonymous function's body: one row, or a column of rows. A row
    /// that would split inside brackets (`-y(1) -y(2)`) gets parentheses.
    pub fn body(&self) -> String {
        let element = |row: &String| {
            let alone = matches!(
                matrust::syntax::parse_expression(&format!("[{row}]")),
                Ok(Expr::Matrix(rows)) if rows.len() == 1 && rows[0].len() == 1
            );
            if alone {
                row.clone()
            } else {
                format!("({row})")
            }
        };
        if self.rows.len() == 1 {
            self.rows[0].clone()
        } else {
            let rows: Vec<String> = self.rows.iter().map(element).collect();
            format!("[{}]", rows.join("; "))
        }
    }
    /// Comment lines that explain a conversion.
    pub fn comment(&self, typed: &str) -> String {
        let Some((_, order)) = &self.converted else {
            return String::new();
        };
        let line = typed.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let map: Vec<String> = (0..*order)
            .map(|k| format!("y({}) = {}", k + 1, self.labels[k]))
            .collect();
        format!(
            "% {} as a first-order system: {}\n",
            line.trim(),
            map.join(", ")
        )
    }
}

/// The code range of one line: no comment, outer blanks or final `;`.
fn code_range(line: &str, scan: &Scan) -> std::ops::Range<usize> {
    let b = line.as_bytes();
    let mut end = scan.end;
    while end > 0 && (b[end - 1].is_ascii_whitespace() || b[end - 1] == b';') {
        end -= 1;
    }
    let mut start = 0;
    while start < end && b[start].is_ascii_whitespace() {
        start += 1;
    }
    start..end
}

/// `name` followed by primes and nothing else, as a left side `x''`.
fn derivative(
    line: &str,
    words: &[Word],
    range: std::ops::Range<usize>,
) -> Option<(String, usize)> {
    let text = &line[range.clone()];
    let start = range.start + (text.len() - text.trim_start().len());
    let end = range.start + text.trim_end().len();
    let inside: Vec<&Word> = words
        .iter()
        .filter(|w| w.start >= start && w.end <= end)
        .collect();
    match inside.as_slice() {
        [w] if w.start == start && w.end + w.primes == end && w.primes > 0 && !w.field => {
            Some((line[w.start..w.end].to_owned(), w.primes))
        }
        _ => None,
    }
}

/// Parses typed equations: right sides of y1'..yn' one per line (an
/// optional `yk' =` in front), one higher-order equation `x'' = ...`, or
/// with `implicit` residuals F(t, y, yp) = 0 in y1.., yp1.. (or yk').
pub fn parse_system(text: &str, implicit: bool) -> Result<System, String> {
    struct Line<'a> {
        text: &'a str,
        number: usize,
        scan: Scan,
        range: std::ops::Range<usize>,
    }
    let mut lines = Vec::new();
    for (k, text) in text.lines().enumerate() {
        let scan = scan(text).map_err(|e| format!("Solve ODE: line {}: {e}", k + 1))?;
        let range = code_range(text, &scan);
        if range.is_empty() {
            continue;
        }
        lines.push(Line {
            text,
            number: k + 1,
            scan,
            range,
        });
    }
    if lines.is_empty() {
        return Err("Solve ODE: type the equations, one per line".into());
    }
    if lines.len() > MAX_EQUATIONS {
        return Err(format!(
            "Solve ODE: at most {MAX_EQUATIONS} equations; use a function file for larger systems"
        ));
    }
    let n = lines.len();
    let lhs = |line: &Line| line.scan.assign.filter(|a| line.range.contains(a));
    let outside = |line: &Line, k: usize| {
        format!(
            "Solve ODE: line {}: y{k} is not one of y1 to y{n}",
            line.number
        )
    };
    let mut converted = None;
    let mut rows = Vec::new();
    if implicit {
        for line in &lines {
            let mut name = |word: &str, primes: usize| -> Result<Option<(String, usize)>, String> {
                if let Some(k) = component(word, "yp") {
                    return if k <= n {
                        Ok(Some((format!("yp({k})"), 0)))
                    } else {
                        Err(outside(line, k).replace(&format!("y{k}"), &format!("yp{k}")))
                    };
                }
                match component(word, "y") {
                    Some(k) if k > n => Err(outside(line, k)),
                    Some(k) if primes > 0 => Ok(Some((format!("yp({k})"), 1))),
                    Some(k) => Ok(Some((format!("y({k})"), 0))),
                    None => Ok(None),
                }
            };
            let row = match lhs(line) {
                Some(a) => {
                    let left =
                        rewrite(line.text, &line.scan.words, line.range.start..a, &mut name)?;
                    let right = rewrite(
                        line.text,
                        &line.scan.words,
                        a + 1..line.range.end,
                        &mut name,
                    )?;
                    if left.is_empty() || right.is_empty() {
                        return Err(format!(
                            "Solve ODE: line {}: a side of = is empty",
                            line.number
                        ));
                    }
                    format!("{left} - ({right})")
                }
                None => rewrite(line.text, &line.scan.words, line.range.clone(), &mut name)?,
            };
            rows.push(row);
        }
    } else if let [line] = lines.as_slice()
        && let Some(a) = lhs(line)
        && let Some((var, order)) = derivative(line.text, &line.scan.words, line.range.start..a)
        && component(&var, "y").is_none()
    {
        // One equation of one variable: x^(m) = g(t, x, .., x^(m-1)).
        if order > MAX_EQUATIONS {
            return Err(format!("Solve ODE: at most order {MAX_EQUATIONS}"));
        }
        let mut name = |word: &str, primes: usize| -> Result<Option<(String, usize)>, String> {
            if word != var {
                return Ok(None);
            }
            if primes < order {
                Ok(Some((format!("y({})", primes + 1), primes)))
            } else {
                Err(format!(
                    "Solve ODE: the right side can use {var} up to {var}{}",
                    "'".repeat(order - 1)
                ))
            }
        };
        let rhs = rewrite(
            line.text,
            &line.scan.words,
            a + 1..line.range.end,
            &mut name,
        )?;
        rows.extend((2..=order).map(|k| format!("y({k})")));
        rows.push(rhs);
        converted = Some((var, order));
    } else {
        let mut slots: Vec<Option<String>> = vec![None; n];
        for (position, line) in lines.iter().enumerate() {
            let (k, start) = match lhs(line) {
                None => (position + 1, line.range.start),
                Some(a) => match derivative(line.text, &line.scan.words, line.range.start..a)
                    .and_then(|(name, primes)| {
                        (primes == 1).then(|| component(&name, "y")).flatten()
                    }) {
                    Some(k) => (k, a + 1),
                    None => {
                        return Err(format!(
                            "Solve ODE: line {}: the left side must be y{}' (or leave it out); \
                             x'' = ... works for one equation of one variable",
                            line.number,
                            position + 1
                        ));
                    }
                },
            };
            if k > n {
                return Err(format!(
                    "Solve ODE: line {}: y{k}' but there are {n} equations",
                    line.number
                ));
            }
            if slots[k - 1].is_some() {
                return Err(format!("Solve ODE: y{k}' is defined twice"));
            }
            let mut name = |word: &str, _: usize| -> Result<Option<(String, usize)>, String> {
                match component(word, "y") {
                    Some(j) if j > n => Err(outside(line, j)),
                    Some(j) => Ok(Some((format!("y({j})"), 0))),
                    None => Ok(None),
                }
            };
            slots[k - 1] = Some(rewrite(
                line.text,
                &line.scan.words,
                start..line.range.end,
                &mut name,
            )?);
        }
        rows = slots.into_iter().map(Option::unwrap_or_default).collect();
    }
    for (k, row) in rows.iter().enumerate() {
        if row.is_empty() {
            return Err(format!("Solve ODE: equation {} is empty", k + 1));
        }
        matrust::syntax::parse_expression(row)
            .map_err(|e| format!("Solve ODE: equation {} does not parse: {e}", k + 1))?;
    }
    let labels = match &converted {
        Some((var, order)) => (0..*order)
            .map(|k| format!("{var}{}", "'".repeat(k)))
            .collect(),
        None => (1..=rows.len()).map(|k| format!("y{k}")).collect(),
    };
    Ok(System {
        rows,
        labels,
        converted,
    })
}

/// Rewrites a typed event value the way the equations name components.
fn event_value(text: &str, system: Option<&System>, n: Option<usize>) -> Result<String, String> {
    let scan = scan(text).map_err(|e| format!("Solve ODE: event condition: {e}"))?;
    let range = code_range(text, &scan);
    if range.is_empty() {
        return Err("Solve ODE: type the event value, for example y1".into());
    }
    let converted = system.and_then(|s| s.converted.clone());
    let mut name = |word: &str, primes: usize| -> Result<Option<(String, usize)>, String> {
        if let Some((var, order)) = &converted
            && word == var
        {
            return if primes < *order {
                Ok(Some((format!("y({})", primes + 1), primes)))
            } else {
                Err(format!(
                    "Solve ODE: the event condition can use {var} up to {var}{}",
                    "'".repeat(order - 1)
                ))
            };
        }
        match component(word, "y") {
            Some(k) if n.is_some_and(|n| k > n) => Err(format!(
                "Solve ODE: the event condition uses y{k}; there are {} components",
                n.unwrap_or_default()
            )),
            Some(k) => Ok(Some((format!("y({k})"), 0))),
            None => Ok(None),
        }
    };
    let value = rewrite(text, &scan.words, range, &mut name)?;
    matrust::syntax::parse_expression(&value)
        .map_err(|e| format!("Solve ODE: the event condition does not parse: {e}"))?;
    Ok(value)
}

// ---------------------------------------------------------------------------
// Values

/// A MATLAB char vector literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Typed numbers: `1 2`, `[1, 2]` or `[1; 2]`.
pub fn numbers(text: &str) -> Option<Vec<f64>> {
    let inner = text.trim();
    let inner = inner
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(inner);
    let values = inner
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().ok())
        .collect::<Option<Vec<f64>>>()?;
    (!values.is_empty()).then_some(values)
}

/// A vector typed as an expression (`[2 0]`, `linspace(0, 1, 11)`) or as
/// bare numbers (`2 0`, which gets brackets).
fn vector_text(text: &str, what: &str) -> Result<String, String> {
    let text = text.trim().trim_end_matches(';').trim();
    if text.is_empty() {
        return Err(format!("Solve ODE: type {what}"));
    }
    match matrust::syntax::parse_expression(text) {
        Ok(_) => {
            super::single_expression(text).map_err(|e| format!("Solve ODE: {what}: {e}"))?;
            Ok(text.to_owned())
        }
        Err(error) => {
            let wrapped = format!("[{text}]");
            match matrust::syntax::parse_expression(&wrapped) {
                Ok(_) => {
                    super::single_expression(&wrapped)
                        .map_err(|e| format!("Solve ODE: {what}: {e}"))?;
                    Ok(wrapped)
                }
                Err(_) => Err(format!("Solve ODE: {what} does not parse: {error}")),
            }
        }
    }
}

const SCALAR_FUNCTIONS: [&str; 12] = [
    "sqrt", "exp", "log", "log10", "sin", "cos", "tan", "asin", "acos", "atan", "abs", "deg2rad",
];
/// An expression that is one number: literals, pi, scalar parameters and
/// scalar functions of them.
fn scalar_like(expr: &Expr, scalars: &BTreeSet<String>) -> bool {
    match expr {
        Expr::Number(_) | Expr::Imaginary(_) => true,
        Expr::Unary(_, a) | Expr::Not(a) | Expr::Transpose(a) | Expr::NonConjugateTranspose(a) => {
            scalar_like(a, scalars)
        }
        Expr::Binary(_, a, b) => scalar_like(a, scalars) && scalar_like(b, scalars),
        Expr::Name(name) => {
            matches!(name.as_str(), "pi" | "Inf" | "inf" | "NaN" | "nan" | "eps")
                || scalars.contains(name)
        }
        Expr::Call(name, args) => {
            SCALAR_FUNCTIONS.contains(&name.as_str())
                && !scalars.contains(name)
                && args.len() == 1
                && scalar_like(&args[0], scalars)
        }
        _ => false,
    }
}
/// How many values a typed vector has, when the text alone says.
pub fn count(text: &str, scalars: &BTreeSet<String>) -> Option<usize> {
    match matrust::syntax::parse_expression(text).ok()? {
        Expr::Matrix(rows) => rows
            .iter()
            .flatten()
            .all(|e| scalar_like(e, scalars))
            .then(|| rows.iter().map(Vec::len).sum()),
        expr => scalar_like(&expr, scalars).then_some(1),
    }
}

/// A blank (default) or positive number.
fn positive(text: &str, what: &str) -> Result<Option<f64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0. => Ok(Some(v)),
        _ => Err(format!("Solve ODE: {what} must be a positive number")),
    }
}
fn vector(values: &[f64]) -> String {
    if values.len() == 1 {
        number(values[0])
    } else {
        let items: Vec<String> = values.iter().map(|v| number(*v)).collect();
        format!("[{}]", items.join(" "))
    }
}

/// Default RelTol and AbsTol of every solver.
pub const DEFAULT_REL_TOL: f64 = 1e-3;
pub const DEFAULT_ABS_TOL: f64 = 1e-6;

// ---------------------------------------------------------------------------
// The solution the private run read back

#[derive(Clone, Debug)]
pub struct Solution {
    /// The solver that ran.
    pub solver: Solver,
    pub t: Vec<f64>,
    /// Points × components, column-major.
    pub y: Vec<f64>,
    pub n: usize,
    pub te: Vec<f64>,
    /// Events × components, column-major.
    pub ye: Vec<f64>,
    pub ie: Vec<f64>,
    /// Steps, failed steps and function evaluations (NaN: not counted).
    pub stats: [f64; 3],
    /// Seconds of the run (solve and plot).
    pub wall: f64,
    pub labels: Vec<String>,
}
impl Solution {
    pub fn points(&self) -> usize {
        self.t.len()
    }
    /// Component k (0-based) at every time.
    pub fn component(&self, k: usize) -> &[f64] {
        let n = self.points();
        &self.y[k * n..(k + 1) * n]
    }
    /// y at the last time.
    pub fn last(&self) -> Vec<f64> {
        (0..self.n)
            .map(|k| *self.component(k).last().unwrap())
            .collect()
    }
    pub fn events(&self) -> usize {
        self.te.len()
    }
    /// Component k of event j.
    pub fn event(&self, j: usize, k: usize) -> f64 {
        self.ye[k * self.events() + j]
    }
    /// A nonstiff solver took very many steps, or long without a count.
    pub fn stiff_hint(&self) -> bool {
        !self.solver.stiff()
            && (self.stats[0] > STIFF_STEPS || (self.stats[0].is_nan() && self.wall > COUNT_LIMIT))
    }
}

fn matrix(outputs: &Outputs, name: &str) -> Result<(usize, usize, Vec<f64>), String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) => Ok((m.rows(), m.cols(), m.data().to_vec())),
        _ => Err(format!("Solve ODE: {name} is not a real array")),
    }
}
fn values(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    matrix(outputs, name).map(|(_, _, v)| v)
}

const PROBE: [&str; 7] = [
    "app_wall",
    "app_t",
    "app_y",
    "app_te",
    "app_ye",
    "app_ie",
    "app_stats",
];

// ---------------------------------------------------------------------------
// The app

/// The pieces the script and the probe share.
struct Call {
    /// Comment, parameters, function and options lines.
    setup: String,
    solver: Solver,
    /// The solver's arguments.
    arguments: String,
    outputs: Vec<(Out, String)>,
    events: bool,
    /// Component count, when the inputs say.
    n: Option<usize>,
    labels: Vec<String>,
    /// Local functions that end the script.
    functions: String,
    /// Names the script assigns besides the outputs.
    created: Vec<String>,
}
impl Call {
    fn name(&self, out: Out) -> &str {
        self.outputs
            .iter()
            .find(|(o, _)| *o == out)
            .map_or("", |(_, name)| name.as_str())
    }
    fn label(&self, k: usize) -> String {
        self.labels
            .get(k)
            .cloned()
            .unwrap_or_else(|| format!("y{}", k + 1))
    }
}

pub struct SolveOde {
    /// The inputs as edited.
    pub spec: Spec,
    /// The inputs of the last Run: what the script uses while auto-run is off.
    pub applied: Spec,
    /// Solve on every change; off, the Run button applies the inputs.
    pub auto: bool,
    pub plot: Plot,
    /// The Script tab is shown instead of the plot.
    pub show_script: bool,
    /// Components (0-based) of the phase plots: x, y and z.
    pub components: [usize; 3],
    /// Export the solution structure instead of t and y.
    pub structure: bool,
    pub names: [Name; 6],
    /// Last good solution; errors keep it on screen.
    pub result: Option<Solution>,
    /// The 3-D view's azimuth and elevation, radians.
    pub angles: (f32, f32),
    /// Run clicks: part of the private prelude, so a click solves again.
    generation: u64,
    /// Stop was clicked: no run until Run or a change of the inputs.
    stopped: bool,
    /// When the current private run started.
    running_since: Option<Instant>,
    /// Workspace variables the script reads (found in `prepare`).
    reads: Vec<String>,
    /// The script as of the last `prepare`, for the Script tab.
    preview: Result<String, String>,
}
impl Default for SolveOde {
    fn default() -> Self {
        let spec = Spec::default();
        Self {
            applied: spec.clone(),
            spec,
            auto: true,
            plot: Plot::Time,
            show_script: false,
            components: [0, 1, 2],
            structure: false,
            names: Default::default(),
            result: None,
            angles: (0.6, 0.45),
            generation: 0,
            stopped: false,
            running_since: None,
            reads: Vec::new(),
            preview: Err(String::new()),
        }
    }
}

impl SolveOde {
    /// The inputs the script uses: as edited while auto-run is on, else as
    /// of the last Run.
    pub fn active(&self) -> &Spec {
        if self.auto { &self.spec } else { &self.applied }
    }
    /// Run: the edited inputs apply and the solve runs again.
    pub fn run(&mut self) {
        self.applied = self.spec.clone();
        self.generation += 1;
        self.stopped = false;
    }
    /// Stop: the running solve is cancelled and no other starts.
    pub fn stop(&mut self) {
        self.stopped = true;
    }
    /// The host's runner state, once per frame.
    pub fn set_running(&mut self, running: bool) {
        if !running {
            self.running_since = None;
        } else if self.running_since.is_none() {
            self.running_since = Some(Instant::now());
        }
    }
    /// The typed system of the active inputs.
    pub fn system(&self) -> Option<Result<System, String>> {
        let spec = self.active();
        (spec.source == Source::Typed).then(|| parse_system(&spec.equations, spec.implicit()))
    }
    /// Checked parameters: name and value.
    fn parameters(&self, converted: Option<&str>) -> Result<Vec<(String, String)>, String> {
        let spec = self.active();
        if spec.source != Source::Typed {
            return Ok(Vec::new());
        }
        let mut out: Vec<(String, String)> = Vec::new();
        for (name, value) in &spec.parameters {
            let name = name.trim();
            let value = value.trim().trim_end_matches(';').trim();
            if name.is_empty() && value.is_empty() {
                continue;
            }
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Solve ODE: the parameter name {name} is not a valid variable name"
                ));
            }
            if matches!(name, "t" | "y" | "yp") || Some(name) == converted {
                return Err(format!(
                    "Solve ODE: {name} is a variable of the equations; give the parameter another name"
                ));
            }
            if out.iter().any(|(n, _)| n == name) {
                return Err(format!("Solve ODE: the parameter {name} is defined twice"));
            }
            if value.is_empty() {
                return Err(format!("Solve ODE: the parameter {name} needs a value"));
            }
            super::single_expression(value)
                .map_err(|e| format!("Solve ODE: the value of {name} does not parse: {e}"))?;
            out.push((name.to_owned(), value.to_owned()));
        }
        if out.len() > MAX_PARAMETERS {
            return Err(format!("Solve ODE: at most {MAX_PARAMETERS} parameters"));
        }
        Ok(out)
    }
    /// Workspace variables the active inputs read.
    fn find_reads(&self, ws: &Snapshot) -> Vec<String> {
        let spec = self.active();
        let parameters: Vec<String> = spec
            .parameters
            .iter()
            .map(|(n, _)| n.trim().to_owned())
            .collect();
        let converted = match self.system() {
            Some(Ok(system)) => system.converted.map(|(var, _)| var),
            _ => None,
        };
        let solver_arguments = ["t", "y", "yp"];
        let mut out: Vec<String> = Vec::new();
        let add = |name: &str, out: &mut Vec<String>| {
            if ws.contains(name)
                && !parameters.iter().any(|p| p == name)
                && !out.iter().any(|n| n == name)
            {
                out.push(name.to_owned());
            }
        };
        let mut words = Vec::new();
        match spec.source {
            Source::Typed => {
                for word in identifiers(&spec.equations) {
                    if !solver_arguments.contains(&word.as_str())
                        && Some(&word) != converted.as_ref()
                    {
                        words.push(word);
                    }
                }
                for (_, value) in &spec.parameters {
                    words.extend(identifiers(value));
                }
            }
            Source::Function => {
                let text = spec.function.trim();
                if text.starts_with('@') {
                    let own = anonymous_parameters(text);
                    words.extend(identifiers(text).into_iter().filter(|w| !own.contains(w)));
                } else {
                    words.push(text.to_owned());
                }
            }
        }
        for text in [&spec.y0, &spec.tspan] {
            words.extend(identifiers(text));
        }
        if spec.implicit() {
            words.extend(identifiers(&spec.yp0));
        }
        if spec.has_mass() {
            words.push(spec.mass.trim().to_owned());
        }
        if spec.has_events() {
            match spec.events {
                Events::Condition => {
                    for word in identifiers(&spec.event_value) {
                        if !solver_arguments.contains(&word.as_str())
                            && Some(&word) != converted.as_ref()
                        {
                            words.push(word);
                        }
                    }
                }
                Events::Function => {
                    let text = spec.event_function.trim();
                    if text.starts_with('@') {
                        let own = anonymous_parameters(text);
                        words.extend(identifiers(text).into_iter().filter(|w| !own.contains(w)));
                    } else {
                        words.push(text.to_owned());
                    }
                }
                Events::None => {}
            }
        }
        for word in words {
            add(&word, &mut out);
        }
        out
    }
    /// The component count the inputs give, else the last solution's.
    pub fn count(&self, ws: &Snapshot) -> Option<usize> {
        self.input_count(ws)
            .or_else(|| self.result.as_ref().map(|r| r.n))
    }
    /// The component count the inputs give.
    fn input_count(&self, _ws: &Snapshot) -> Option<usize> {
        match self.system() {
            Some(Ok(system)) => Some(system.rows.len()),
            Some(Err(_)) => None,
            None => count(self.active().y0.trim(), &BTreeSet::new()),
        }
    }

    /// The outputs the script leaves.
    fn used_outputs(&self) -> Vec<Out> {
        if self.structure {
            vec![Out::Sol]
        } else if self.active().has_events() {
            vec![Out::T, Out::Y, Out::Te, Out::Ye, Out::Ie]
        } else {
            vec![Out::T, Out::Y]
        }
    }
    fn parameter_names(&self) -> Vec<String> {
        self.active()
            .parameters
            .iter()
            .map(|(n, _)| n.trim().to_owned())
            .collect()
    }
    /// Output names: the edited ones, else unused defaults (unchecked).
    fn default_names(&self, ws: &Snapshot) -> Vec<(Out, String)> {
        let parameters = self.parameter_names();
        let used = self.used_outputs();
        let edited: Vec<String> = used
            .iter()
            .filter(|o| self.names[o.index()].edited)
            .map(|o| self.names[o.index()].text.trim().to_owned())
            .collect();
        let mut chosen: Vec<(Out, String)> = Vec::new();
        for out in used {
            let entry = &self.names[out.index()];
            let name = if entry.edited {
                entry.text.trim().to_owned()
            } else {
                unused_name(out.base(), |n| {
                    ws.contains(n)
                        || parameters.iter().any(|p| p == n)
                        || edited.iter().any(|e| e == n)
                        || chosen.iter().any(|(_, c)| c == n)
                })
            };
            chosen.push((out, name));
        }
        chosen
    }
    /// Output names: the edited ones, else unused defaults.
    pub fn output_names(&self, ws: &Snapshot) -> Result<Vec<(Out, String)>, String> {
        let parameters = self.parameter_names();
        let chosen = self.default_names(ws);
        for (out, name) in &chosen {
            let what = out.label().trim_end_matches(" name").to_lowercase();
            super::check_export_name(name)?;
            if !matrust::files::valid_name(name) {
                return Err(format!(
                    "Solve ODE: the {what} name is not a valid variable name"
                ));
            }
            if parameters.iter().any(|p| p == name) {
                return Err(format!(
                    "Solve ODE: {name} is a parameter; choose another {what} name"
                ));
            }
            if self.reads.iter().any(|r| r == name) {
                return Err(format!(
                    "Solve ODE: the solve reads the workspace variable {name}; choose another {what} name"
                ));
            }
        }
        for (i, (_, a)) in chosen.iter().enumerate() {
            if chosen[..i].iter().any(|(_, b)| a == b) {
                return Err("Solve ODE: the outputs need different names".into());
            }
        }
        Ok(chosen)
    }

    /// Everything the script and the probe need, checked.
    fn call(&self, ws: &Snapshot) -> Result<Call, String> {
        let spec = self.active();
        let solver = spec.resolved();
        let implicit = spec.implicit();
        let system = self.system().transpose()?;
        let converted = system
            .as_ref()
            .and_then(|s| s.converted.as_ref().map(|(var, _)| var.as_str()));
        let parameters = self.parameters(converted)?;
        let outputs = self.output_names(ws)?;
        let mut scalars = BTreeSet::new();
        for (name, value) in &parameters {
            if count(value, &scalars) == Some(1) {
                scalars.insert(name.clone());
            } else {
                scalars.remove(name);
            }
        }
        let taken = |n: &str, extra: &[&str]| {
            ws.contains(n)
                || parameters.iter().any(|(p, _)| p == n)
                || outputs.iter().any(|(_, o)| o == n)
                || self.reads.iter().any(|r| r == n)
                || extra.contains(&n)
        };
        let mut setup = String::new();
        let mut created: Vec<String> = parameters.iter().map(|(n, _)| n.clone()).collect();
        // The function.
        let mut n = None;
        let mut labels = Vec::new();
        let function = match (&system, spec.source) {
            (Some(system), _) => {
                setup.push_str(&system.comment(&spec.equations));
                for (name, value) in &parameters {
                    setup.push_str(&format!("{name} = {value};\n"));
                }
                let f = unused_name("f", |c| taken(c, &[]));
                let arguments = if implicit { "t, y, yp" } else { "t, y" };
                setup.push_str(&format!("{f} = @({arguments}) {};\n", system.body()));
                n = Some(system.rows.len());
                labels = system.labels.clone();
                created.push(f.clone());
                f
            }
            (None, _) => {
                let text = spec.function.trim().trim_end_matches(';').trim();
                if text.is_empty() {
                    return Err(
                        "Solve ODE: type a function name, a handle variable or @(t, y) ...".into(),
                    );
                }
                if text.starts_with('@') {
                    super::single_expression(text)
                        .map_err(|e| format!("Solve ODE: the function does not parse: {e}"))?;
                    let f = unused_name("f", |c| taken(c, &[]));
                    setup.push_str(&format!("{f} = {text};\n"));
                    created.push(f.clone());
                    f
                } else if let Some(array) = ws.get(text) {
                    if !matches!(array, Array::Handle(_)) {
                        return Err(format!(
                            "Solve ODE: the workspace variable {text} is not a function handle"
                        ));
                    }
                    setup.push_str(&format!(
                        "% {text} is the workspace function handle {}.\n",
                        if implicit { "F(t, y, yp)" } else { "f(t, y)" }
                    ));
                    text.to_owned()
                } else if matrust::files::valid_name(text) {
                    format!("@{text}")
                } else {
                    return Err(format!(
                        "Solve ODE: {text} is not a function name, a handle variable or @(t, y) ..."
                    ));
                }
            }
        };
        // Initial values and times.
        let y0 = vector_text(&spec.y0, "the initial values y0")?;
        let tspan = vector_text(&spec.tspan, "the time span")?;
        if let Some(times) = numbers(&tspan) {
            if times.len() < 2 {
                return Err("Solve ODE: the time span needs two values, [t0 tf]".into());
            }
            if times.iter().any(|v| !v.is_finite()) {
                return Err("Solve ODE: the times must be finite".into());
            }
            let rising = times[1] > times[0];
            if times
                .windows(2)
                .any(|w| (w[1] > w[0]) != rising || w[1] == w[0])
            {
                return Err("Solve ODE: the times must all increase (or all decrease)".into());
            }
        }
        let given = count(&y0, &scalars);
        match (n, given) {
            (Some(n), Some(m)) if n != m => {
                return Err(format!(
                    "Solve ODE: y0 has {m} value{} but there {} {n} equation{}",
                    if m == 1 { "" } else { "s" },
                    if n == 1 { "is" } else { "are" },
                    if n == 1 { "" } else { "s" }
                ));
            }
            (None, Some(m)) => n = Some(m),
            _ => {}
        }
        if labels.is_empty()
            && let Some(n) = n
        {
            labels = (1..=n).map(|k| format!("y{k}")).collect();
        }
        let mut arguments = format!("{function}, {tspan}, {y0}");
        if implicit {
            let yp0 = vector_text(&spec.yp0, "the initial slopes yp0")?;
            if let (Some(n), Some(m)) = (n, count(&yp0, &scalars))
                && n != m
            {
                return Err(format!(
                    "Solve ODE: yp0 has {m} values but y0 has {n}; give one slope per component"
                ));
            }
            arguments.push_str(&format!(", {yp0}"));
        }
        // Options that differ from the defaults.
        let mut options: Vec<String> = Vec::new();
        if let Some(r) = positive(&spec.rel_tol, "RelTol")?
            && r != DEFAULT_REL_TOL
        {
            options.push(format!("'RelTol', {}", number(r)));
        }
        if !spec.abs_tol.trim().is_empty() {
            let tolerances = numbers(&spec.abs_tol)
                .filter(|v| v.iter().all(|x| x.is_finite() && *x > 0.))
                .ok_or("Solve ODE: AbsTol must be positive numbers")?;
            if tolerances.len() > 1 && n.is_some_and(|n| n != tolerances.len()) {
                return Err(format!(
                    "Solve ODE: AbsTol needs one value or one per component ({})",
                    n.unwrap_or_default()
                ));
            }
            if tolerances != [DEFAULT_ABS_TOL] {
                options.push(format!("'AbsTol', {}", vector(&tolerances)));
            }
        }
        if let Some(h) = positive(&spec.max_step, "MaxStep")? {
            options.push(format!("'MaxStep', {}", number(h)));
        }
        if let Some(h) = positive(&spec.initial_step, "InitialStep")? {
            options.push(format!("'InitialStep', {}", number(h)));
        }
        if !spec.non_negative.trim().is_empty() {
            let indices = numbers(&spec.non_negative)
                .filter(|v| v.iter().all(|k| k.fract() == 0. && *k >= 1.))
                .ok_or("Solve ODE: NonNegative takes component numbers such as 1 2")?;
            if let Some(n) = n
                && indices.iter().any(|k| *k > n as f64)
            {
                return Err(format!(
                    "Solve ODE: NonNegative names a component above {n}"
                ));
            }
            options.push(format!("'NonNegative', {}", vector(&indices)));
        }
        if spec.has_mass() {
            let mass = spec.mass.trim();
            match ws.get(mass) {
                Some(Array::Numeric(m)) if m.rows() == m.cols() && m.rows() > 0 => {
                    if let Some(n) = n
                        && m.rows() != n
                    {
                        return Err(format!(
                            "Solve ODE: the mass matrix {mass} is {0}-by-{0}; the system has {n} components",
                            m.rows()
                        ));
                    }
                }
                Some(_) => {
                    return Err(format!(
                        "Solve ODE: the mass matrix {mass} is not a real square matrix"
                    ));
                }
                None => {
                    return Err(format!(
                        "Solve ODE: workspace variable {mass} does not exist"
                    ));
                }
            }
            options.push(format!("'Mass', {mass}"));
        }
        let mut functions = String::new();
        let events = spec.has_events();
        if events {
            let handle = match spec.events {
                Events::Condition => {
                    let value = event_value(&spec.event_value, system.as_ref(), n)?;
                    // Parameters and workspace values the condition uses
                    // reach the local function as arguments.
                    let extra: Vec<String> = identifiers(&value)
                        .into_iter()
                        .filter(|w| {
                            parameters.iter().any(|(p, _)| p == w)
                                || (ws.contains(w) && !matches!(w.as_str(), "t" | "y"))
                        })
                        .collect();
                    let list = std::iter::once("t, y".to_owned())
                        .chain(extra.iter().cloned())
                        .collect::<Vec<_>>()
                        .join(", ");
                    functions = format!(
                        "function [value, isterminal, direction] = {EVENT_FUNCTION}({list})\n\
                         % An event is a zero of value.\n\
                         value = {value};\n\
                         isterminal = {};\n\
                         direction = {};\n\
                         end\n",
                        u8::from(spec.terminal),
                        spec.direction.value()
                    );
                    if extra.is_empty() {
                        format!("@{EVENT_FUNCTION}")
                    } else {
                        format!("@(t, y) {EVENT_FUNCTION}({list})")
                    }
                }
                _ => {
                    let text = spec.event_function.trim().trim_end_matches(';').trim();
                    if text.is_empty() {
                        return Err(
                            "Solve ODE: type the event function's name or handle variable".into(),
                        );
                    }
                    if text.starts_with('@') {
                        super::single_expression(text).map_err(|e| {
                            format!("Solve ODE: the event function does not parse: {e}")
                        })?;
                        text.to_owned()
                    } else if let Some(array) = ws.get(text) {
                        if !matches!(array, Array::Handle(_)) {
                            return Err(format!(
                                "Solve ODE: the workspace variable {text} is not a function handle"
                            ));
                        }
                        text.to_owned()
                    } else if matrust::files::valid_name(text) {
                        format!("@{text}")
                    } else {
                        return Err(format!(
                            "Solve ODE: {text} is not a function name or a handle variable"
                        ));
                    }
                }
            };
            options.push(format!("'Events', {handle}"));
        }
        if !options.is_empty() {
            let opts = unused_name("opts", |c| taken(c, &[function.as_str()]));
            setup.push_str(&format!("{opts} = odeset({});\n", options.join(", ")));
            arguments.push_str(&format!(", {opts}"));
            created.push(opts);
        }
        Ok(Call {
            setup,
            solver,
            arguments,
            outputs,
            events,
            n,
            labels,
            functions,
            created,
        })
    }

    /// The plot lines of the script.
    fn plot_lines(&self, call: &Call) -> Result<String, String> {
        let needs = self.plot.needs();
        if let Some(n) = call.n
            && n < needs
        {
            return Err(format!(
                "Solve ODE: the {} needs {needs} components; this system has {n}",
                self.plot.label().to_lowercase()
            ));
        }
        let [i, j, k] = self.components.map(|c| c + 1);
        if let Some(n) = call.n
            && self.components[..needs].iter().any(|c| *c >= n)
        {
            return Err(format!("Solve ODE: the plot shows a component above {n}"));
        }
        // How the script names the solution and the events.
        let (t, y, column, ye, event_column, event_times) = if self.structure {
            let sol = call.name(Out::Sol);
            (
                format!("{sol}.x"),
                format!("{sol}.y"),
                format!("{sol}.y(%, :)"),
                format!("{sol}.ye"),
                format!("{sol}.ye(%, :)"),
                format!("repmat({sol}.xe, size({sol}.ye, 1), 1)"),
            )
        } else {
            let (t, y) = (call.name(Out::T), call.name(Out::Y));
            let (te, ye) = (call.name(Out::Te), call.name(Out::Ye));
            (
                t.to_owned(),
                y.to_owned(),
                format!("{y}(:, %)"),
                ye.to_owned(),
                format!("{ye}(:, %)"),
                format!("repmat({te}, 1, size({ye}, 2))"),
            )
        };
        let col = |k: usize| column.replace('%', &k.to_string());
        let event_col = |k: usize| event_column.replace('%', &k.to_string());
        let mut out = String::new();
        let marks = |line: String| format!("hold on\n{line}\nhold off\n");
        match self.plot {
            Plot::Time => {
                out.push_str(&format!("plot({t}, {y})\nxlabel('t')\n"));
                if !call.labels.is_empty() {
                    // Before the event marks, which the legend leaves out.
                    let mut labels: Vec<String> = call.labels.iter().map(|l| quote(l)).collect();
                    if call.events {
                        labels.push("'AutoUpdate', 'off'".into());
                    }
                    out.push_str(&format!("legend({})\n", labels.join(", ")));
                }
                if call.events {
                    out.push_str(&marks(format!("plot({event_times}, {ye}, 'o')")));
                }
            }
            Plot::Phase => {
                out.push_str(&format!("plot({}, {})\n", col(i), col(j)));
                if call.events {
                    out.push_str(&marks(format!(
                        "plot({}, {}, 'o')",
                        event_col(i),
                        event_col(j)
                    )));
                }
                out.push_str(&format!(
                    "xlabel({})\nylabel({})\n",
                    quote(&call.label(i - 1)),
                    quote(&call.label(j - 1))
                ));
            }
            Plot::Phase3 => {
                out.push_str(&format!("plot3({}, {}, {})\n", col(i), col(j), col(k)));
                if call.events {
                    out.push_str(&marks(format!(
                        "plot3({}, {}, {}, 'o')",
                        event_col(i),
                        event_col(j),
                        event_col(k)
                    )));
                }
                out.push_str(&format!(
                    "xlabel({})\nylabel({})\nzlabel({})\ngrid on\n",
                    quote(&call.label(i - 1)),
                    quote(&call.label(j - 1)),
                    quote(&call.label(k - 1))
                ));
            }
        }
        Ok(out)
    }

    /// The script, whether or not Stop was clicked.
    pub fn build(&self, ws: &Snapshot) -> Result<String, String> {
        let call = self.call(ws)?;
        let outputs: Vec<&str> = call.outputs.iter().map(|(_, n)| n.as_str()).collect();
        let mut script = call.setup.clone();
        let left = if outputs.len() == 1 {
            outputs[0].to_owned()
        } else {
            format!("[{}]", outputs.join(", "))
        };
        script.push_str(&format!(
            "{left} = {}({});\n",
            call.solver.name(),
            call.arguments
        ));
        script.push_str(&self.plot_lines(&call)?);
        if !call.functions.is_empty() {
            script.push('\n');
            script.push_str(&call.functions);
        }
        Ok(script)
    }
}

/// The parameter names of `@(a, b) ...`.
fn anonymous_parameters(text: &str) -> Vec<String> {
    match matrust::syntax::parse_expression(text) {
        Ok(Expr::Anonymous(f)) => f.params.clone(),
        _ => Vec::new(),
    }
}

impl App for SolveOde {
    fn kind(&self) -> AppKind {
        AppKind::SolveOde
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.stopped {
            return Err("Solve ODE: stopped. Press Run to solve again.".into());
        }
        self.build(ws)
    }
    fn inputs(&self) -> Vec<String> {
        self.reads.clone()
    }
    fn prelude(&self, ws: &Snapshot) -> String {
        let mut code = String::new();
        if let Some(folder) = ws.folder() {
            // Function files of the current folder resolve as in the workspace.
            code.push_str(&format!(
                "try\ncd({});\nend\n",
                quote(&folder.to_string_lossy())
            ));
        }
        if self.generation > 0 {
            code.push_str(&format!("% run {}\n", self.generation));
        }
        code.push_str("app_tic = tic;\n");
        code
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(call) = self.call(ws) else {
            return (String::new(), Vec::new());
        };
        let mut code = String::from("app_wall = toc(app_tic);\n");
        if self.structure {
            code.push_str(&format!(
                "app_s = {};\n[app_t, app_y] = deal(app_s.x(:), app_s.y.');\n",
                call.name(Out::Sol)
            ));
            if call.events {
                code.push_str(
                    "[app_te, app_ye, app_ie] = deal(app_s.xe(:), app_s.ye.', app_s.ie(:));\n",
                );
            }
            code.push_str(
                "app_stats = [app_s.stats.nsteps, app_s.stats.nfailed, app_s.stats.nfevals];\n",
            );
        } else {
            code.push_str(&format!(
                "[app_t, app_y] = deal({}(:), {});\n",
                call.name(Out::T),
                call.name(Out::Y)
            ));
            if call.events {
                code.push_str(&format!(
                    "[app_te, app_ye, app_ie] = deal({}(:), {}, {}(:));\n",
                    call.name(Out::Te),
                    call.name(Out::Ye),
                    call.name(Out::Ie)
                ));
            }
            // [t, y] has no counts: a second solve with the structure
            // output reads them, unless the first was slow.
            code.push_str(&format!(
                "app_stats = NaN(1, 3);\n\
                 if app_wall < {}\n\
                 app_s = {}({});\n\
                 app_stats = [app_s.stats.nsteps, app_s.stats.nfailed, app_s.stats.nfevals];\n\
                 end\n",
                number(COUNT_LIMIT),
                call.solver.name(),
                call.arguments
            ));
        }
        if !call.events {
            code.push_str("app_te = []; app_ye = []; app_ie = [];\n");
        }
        (code, PROBE.iter().map(|s| (*s).to_owned()).collect())
    }
    fn prepare(&mut self, ws: &Snapshot) {
        if self.auto && self.applied != self.spec {
            self.applied = self.spec.clone();
            self.stopped = false;
        }
        self.reads = self.find_reads(ws);
        if let Some(n) = self.count(ws) {
            // Out-of-range picks fall back, keeping the axes apart.
            for (slot, c) in self.components.iter_mut().enumerate() {
                if *c >= n {
                    *c = slot.min(n.saturating_sub(1));
                }
            }
        }
        self.preview = self.build(ws);
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let t = values(outputs, "app_t")?;
        let (rows, n, y) = matrix(outputs, "app_y")?;
        if t.is_empty() || n == 0 {
            return Err("Solve ODE: the solver returned no points".into());
        }
        if rows != t.len() {
            return Err(format!(
                "Solve ODE: the solution has {rows} rows for {} times",
                t.len()
            ));
        }
        let te = values(outputs, "app_te")?;
        let (events, width, ye) = matrix(outputs, "app_ye")?;
        let ie = values(outputs, "app_ie")?;
        let (ye, ie) = if te.is_empty() || events != te.len() || width != n {
            (Vec::new(), Vec::new())
        } else {
            (ye, ie)
        };
        let te = if ye.is_empty() { Vec::new() } else { te };
        let stats = values(outputs, "app_stats")?;
        let stats = if stats.len() == 3 {
            [stats[0], stats[1], stats[2]]
        } else {
            [f64::NAN; 3]
        };
        let wall = values(outputs, "app_wall")?
            .first()
            .copied()
            .unwrap_or(f64::NAN);
        let labels = match self.system() {
            Some(Ok(system)) if system.labels.len() == n => system.labels,
            _ => (1..=n).map(|k| format!("y{k}")).collect(),
        };
        self.result = Some(Solution {
            solver: self.active().resolved(),
            t,
            y,
            n,
            te,
            ye,
            ie,
            stats,
            wall,
            labels,
        });
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        let width = ui.available_width();
        let handles = handles(ws);
        let error = |ui: &mut egui::Ui, text: &str| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text.trim_start_matches("Solve ODE: "))
                        .size(crate::tokens::CAPTION)
                        .color(p.error),
                )
                .wrap(),
            );
        };
        let spec = &mut self.spec;
        let implicit = spec.implicit();
        field(ui, "Equations", |ui| {
            ui.horizontal(|ui| {
                if crate::tokens::toggle(ui, spec.source == Source::Typed, "Typed").clicked() {
                    spec.source = Source::Typed;
                }
                if crate::tokens::toggle(ui, spec.source == Source::Function, "Function").clicked()
                {
                    spec.source = Source::Function;
                }
            });
        });
        match spec.source {
            Source::Typed => {
                ui.label(if implicit {
                    "F(t, y, yp) = 0, one residual per line"
                } else {
                    "dy/dt, one equation per line"
                });
                ui.add(
                    egui::TextEdit::multiline(&mut spec.equations)
                        .id_salt("solve_ode_equations")
                        .code_editor()
                        .desired_rows(4)
                        .desired_width(width)
                        .char_limit(16_384),
                );
                match parse_system(&spec.equations, implicit) {
                    Ok(system) => {
                        let mut lines = Vec::new();
                        if let Some((var, _)) = &system.converted {
                            lines.push(format!(
                                "{var} as a system: {}",
                                (0..system.labels.len())
                                    .map(|k| format!("y{} = {}", k + 1, system.labels[k]))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                        for (k, row) in system.rows.iter().enumerate().take(SHOWN_ROWS) {
                            lines.push(if implicit {
                                format!("0 = {row}")
                            } else {
                                format!("y{}' = {row}", k + 1)
                            });
                        }
                        if system.rows.len() > SHOWN_ROWS {
                            lines.push(format!("… {} more", system.rows.len() - SHOWN_ROWS));
                        }
                        crate::tokens::helper(ui, lines.join("\n"));
                    }
                    Err(text) => error(ui, &text),
                }
                crate::tokens::helper(
                    ui,
                    if implicit {
                        "Use t, y1, y2, … and yp1, yp2, … (or y1'); a = b means a - b = 0."
                    } else {
                        "Use t and y1, y2, …; or one line such as x'' = -x."
                    },
                );
                ui.add_space(crate::tokens::S);
                parameters_ui(ui, spec, ws);
            }
            Source::Function => {
                field(ui, "Function", |ui| {
                    ui.add(
                        crate::tokens::input(
                            egui::TextEdit::singleline(&mut spec.function).char_limit(4096),
                        )
                        .hint_text("name, handle variable or @(t, y) …")
                        .desired_width(width),
                    );
                    handle_picker(
                        ui,
                        "solve_ode_function_handles",
                        &mut spec.function,
                        &handles,
                    );
                    crate::tokens::helper(
                        ui,
                        if implicit {
                            "F(t, y, yp) returns the residual column."
                        } else {
                            "f(t, y) returns the column dy/dt. Files come from the current folder."
                        },
                    );
                });
            }
        }
        section(ui);
        let text_field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str| {
            field(ui, label, |ui| {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(text).char_limit(4096))
                        .hint_text(hint)
                        .desired_width(width),
                );
                super::expression_message(ui, vector_text(text, label).map(|_| ()));
            });
        };
        text_field(ui, "Initial values y0", &mut spec.y0, "[1 0]");
        if implicit {
            text_field(ui, "Initial slopes yp0", &mut spec.yp0, "[0 1]");
        }
        text_field(ui, "Time", &mut spec.tspan, "[0 10]");
        crate::tokens::helper(ui, "[t0 tf], or the output times such as 0:0.1:10");
        section(ui);
        field(ui, "Solver", |ui| {
            egui::ComboBox::from_id_salt("solve_ode_solver")
                .width(width)
                .selected_text(spec.solver.label())
                .show_ui(ui, |ui| {
                    for solver in Solver::ALL {
                        ui.selectable_value(&mut spec.solver, solver, solver.label());
                    }
                });
            if spec.solver == Solver::Auto {
                ui.checkbox(&mut spec.stiff, "Stiff problem");
                let why = if spec.stiff {
                    "for a stiff problem"
                } else if spec.events != Events::None {
                    "(ode45 has no event location yet)"
                } else {
                    "for a nonstiff problem"
                };
                crate::tokens::helper(ui, format!("Uses {} {why}", spec.resolved().name()));
            }
        });
        if spec.resolved().mass() {
            let n = match parse_system(&spec.equations, false) {
                Ok(system) if spec.source == Source::Typed => Some(system.rows.len()),
                _ => count(spec.y0.trim(), &BTreeSet::new()),
            };
            let matrices = square_matrices(ws, n);
            field(ui, "Mass matrix", |ui| {
                let shown = if spec.mass.is_empty() {
                    "None (identity)".to_owned()
                } else {
                    spec.mass.clone()
                };
                egui::ComboBox::from_id_salt("solve_ode_mass")
                    .width(width)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut spec.mass, String::new(), "None (identity)");
                        for (name, size) in &matrices {
                            ui.selectable_value(
                                &mut spec.mass,
                                name.clone(),
                                format!("{name} ({size}-by-{size})"),
                            );
                        }
                    });
                crate::tokens::helper(ui, "M(t, y) y' = f(t, y) with a constant M");
            });
        }
        section(ui);
        ui.label("Options");
        crate::tokens::helper(ui, "Blank uses the default; the script lists only changes.");
        let options: [(&str, &mut String, &str); 5] = [
            ("RelTol", &mut spec.rel_tol, "1e-3"),
            ("AbsTol", &mut spec.abs_tol, "1e-6"),
            ("MaxStep", &mut spec.max_step, "|tf - t0| / 10"),
            ("InitialStep", &mut spec.initial_step, "automatic"),
            ("NonNegative", &mut spec.non_negative, "none"),
        ];
        egui::Grid::new("solve_ode_options")
            .num_columns(2)
            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
            .show(ui, |ui| {
                for (label, text, hint) in options {
                    ui.label(label);
                    ui.add(
                        crate::tokens::input(egui::TextEdit::singleline(text).char_limit(1024))
                            .hint_text(hint)
                            .desired_width((width - 96.).max(80.)),
                    );
                    ui.end_row();
                }
            });
        if !spec.non_negative.trim().is_empty()
            && (matches!(spec.solver, Solver::Ode23s | Solver::Ode15i)
                || !spec.mass.trim().is_empty())
        {
            crate::tokens::helper(
                ui,
                "This solver or mass-matrix combination does not support NonNegative.",
            );
        }
        if !implicit {
            section(ui);
            events_ui(ui, spec, &handles);
        }
        section(ui);
        ui.checkbox(&mut self.auto, "Solve on every change");
        ui.add_space(crate::tokens::XS);
        let run = if self.auto { "Run again" } else { "Run" };
        if ui
            .add(
                egui::Button::new(run)
                    .min_size(egui::vec2(ui.available_width(), crate::tokens::CONTROL)),
            )
            .clicked()
        {
            self.run();
        }
        if !self.auto && self.applied != self.spec {
            crate::tokens::helper(ui, "The inputs changed since the last run.");
        }
    }
    fn main_caption(&self) -> &'static str {
        "SOLUTION"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for plot in Plot::ALL {
                let tab =
                    crate::tokens::tab(ui, !self.show_script && self.plot == plot, plot.label());
                if plot == Plot::Time {
                    crate::focus_areas::register(
                        ui,
                        &tab,
                        crate::focus_areas::Area::App(AppKind::SolveOde),
                    );
                }
                if tab.clicked() {
                    self.plot = plot;
                    self.show_script = false;
                }
            }
            if crate::tokens::tab(ui, self.show_script, "Script").clicked() {
                self.show_script = true;
            }
        });
        ui.add_space(crate::tokens::S);
        if self.show_script {
            let mut text = match &self.preview {
                Ok(script) => script.clone(),
                Err(error) => format!("% {}", error.trim_start_matches("Solve ODE: ")),
            };
            egui::ScrollArea::both()
                .id_salt("solve_ode_script")
                .auto_shrink([false, false])
                .show_themed(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .interactive(false)
                            .desired_width(f32::INFINITY),
                    );
                });
            return;
        }
        let labels: Vec<String> = match &self.result {
            Some(r) => r.labels.clone(),
            None => Vec::new(),
        };
        if self.plot != Plot::Time && !labels.is_empty() {
            ui.horizontal_wrapped(|ui| {
                let axes = ["x", "y", "z"];
                for (axis, slot) in axes
                    .iter()
                    .zip(self.components.iter_mut())
                    .take(self.plot.needs())
                {
                    ui.label(*axis);
                    egui::ComboBox::from_id_salt(("solve_ode_component", *axis))
                        .width(96.)
                        .selected_text(labels.get(*slot).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (k, label) in labels.iter().enumerate() {
                                ui.selectable_value(slot, k, label.as_str());
                            }
                        });
                }
            });
            ui.add_space(crate::tokens::XS);
        }
        let solver = self.active().resolved();
        if let Some(since) = self.running_since {
            let seconds = since.elapsed().as_secs_f64();
            if seconds > 0.5 {
                ui.horizontal(|ui| {
                    crate::tokens::helper(
                        ui,
                        format!("Solving with {}… {:.0} s", solver.name(), seconds),
                    );
                    if ui.button("Stop").clicked() {
                        self.stop();
                    }
                });
                if seconds > SLOW_SECONDS && !solver.stiff() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{} is slow here: the problem may be stiff. Tick Stiff problem or \
                                 choose ode15s.",
                                solver.name()
                            ))
                            .color(p.warning),
                        )
                        .wrap(),
                    );
                }
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        let Some(r) = &self.result else {
            crate::tokens::helper(ui, "No solution yet: the status line says why.");
            return;
        };
        if r.stiff_hint() && self.running_since.is_none() {
            let steps = if r.stats[0].is_nan() {
                format!("{} ran for {} s", r.solver.name(), short(r.wall))
            } else {
                format!("{} took {} steps", r.solver.name(), short(r.stats[0]))
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{steps}: the problem may be stiff. Tick Stiff problem or choose ode15s."
                    ))
                    .color(p.warning),
                )
                .wrap(),
            );
        }
        let height = (ui.available_height() - crate::tokens::S).max(160.);
        match self.plot {
            Plot::Time => time_plot(ui, height, r),
            Plot::Phase => phase_plot(ui, height, r, [self.components[0], self.components[1]]),
            Plot::Phase3 => phase3_plot(ui, height, r, self.components, &mut self.angles),
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(r) = &self.result else {
            return ["Solver", "Steps", "Failed steps", "Function evaluations"]
                .into_iter()
                .map(|label| (label, "—".to_owned()))
                .collect();
        };
        let counted = |v: f64| {
            if v.is_nan() {
                "— (slow solve)".to_owned()
            } else {
                short(v)
            }
        };
        vec![
            ("Solver", r.solver.name().to_owned()),
            ("Steps", counted(r.stats[0])),
            ("Failed steps", counted(r.stats[1])),
            ("Function evaluations", counted(r.stats[2])),
            ("Points", r.points().to_string()),
            ("Final time", short(*r.t.last().unwrap())),
            ("Events", r.events().to_string()),
            ("Wall time", wall_time(r.wall)),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let p = crate::tokens::palette(ui.ctx());
        if let Some(r) = &self.result {
            ui.label(egui::RichText::new("Final state y(tf)").color(p.text_muted));
            egui::Grid::new("solve_ode_final")
                .num_columns(2)
                .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                .show(ui, |ui| {
                    for (label, value) in r.labels.iter().zip(r.last()).take(SHOWN_ROWS) {
                        ui.label(egui::RichText::new(label).color(p.text_muted));
                        ui.label(egui::RichText::new(short(value)).color(p.text));
                        ui.end_row();
                    }
                });
            if r.n > SHOWN_ROWS {
                crate::tokens::helper(ui, format!("… {} more in the export", r.n - SHOWN_ROWS));
            }
            if self.active().has_events() {
                ui.add_space(crate::tokens::S);
                ui.label(egui::RichText::new("Events (te, ie, ye)").color(p.text_muted));
                if r.events() == 0 {
                    crate::tokens::helper(ui, "None in the time span");
                }
                egui::Grid::new("solve_ode_events")
                    .num_columns(3)
                    .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
                    .show(ui, |ui| {
                        for j in 0..r.events().min(SHOWN_ROWS) {
                            ui.label(short(r.te[j]));
                            ui.label(short(r.ie.get(j).copied().unwrap_or(f64::NAN)));
                            let states: Vec<String> =
                                (0..r.n.min(4)).map(|k| short(r.event(j, k))).collect();
                            ui.label(format!(
                                "{}{}",
                                states.join(", "),
                                if r.n > 4 { ", …" } else { "" }
                            ));
                            ui.end_row();
                        }
                    });
            }
            ui.add_space(crate::tokens::M);
        }
        ui.checkbox(&mut self.structure, "Structure output (sol)");
        crate::tokens::helper(ui, "sol works with deval and odextend.");
        ui.add_space(crate::tokens::XS);
        for (out, default) in self.default_names(ws) {
            let entry = &mut self.names[out.index()];
            if !entry.edited {
                entry.text = default;
            }
            let before = entry.text.clone();
            name_field(ui, out.label(), &mut entry.text, ws);
            entry.edited |= entry.text != before;
        }
    }
    fn exported(&self, ws: &Snapshot) -> Vec<String> {
        match self.call(ws) {
            Ok(call) => call
                .outputs
                .iter()
                .map(|(_, n)| n.clone())
                .chain(call.created)
                .collect(),
            Err(_) => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        let r = self.result.as_ref()?;
        let last = r.last();
        let shown: Vec<String> = last.iter().take(4).map(|v| short(*v)).collect();
        let mut text = format!(
            "{} · y({}) = [{}{}]",
            r.solver.name(),
            short(*r.t.last().unwrap()),
            shown.join(", "),
            if r.n > 4 { ", …" } else { "" }
        );
        if !r.stats[0].is_nan() {
            text.push_str(&format!(" · {} steps", short(r.stats[0])));
        }
        if r.events() > 0 {
            text.push_str(&format!(" · {} events", r.events()));
        }
        text.push_str(&format!(" · {}", wall_time(r.wall)));
        if r.stiff_hint() {
            text.push_str(" · may be stiff: try ode15s");
        }
        Some(text)
    }
}

fn wall_time(seconds: f64) -> String {
    if !seconds.is_finite() {
        "—".into()
    } else if seconds < 1. {
        format!("{} ms", short(seconds * 1e3))
    } else {
        format!("{} s", short(seconds))
    }
}

// ---------------------------------------------------------------------------
// Inputs

fn section(ui: &mut egui::Ui) {
    ui.add_space(crate::tokens::XS);
    ui.separator();
    ui.add_space(crate::tokens::XS);
}

/// Workspace function handles.
pub fn handles(ws: &Snapshot) -> Vec<String> {
    ws.variables()
        .filter(|(_, a)| matches!(a, Array::Handle(_)))
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// Real square matrices (of size n when known): name and size.
pub fn square_matrices(ws: &Snapshot, n: Option<usize>) -> Vec<(String, usize)> {
    ws.variables()
        .filter_map(|(name, array)| match array {
            Array::Numeric(m)
                if m.rows() == m.cols() && m.rows() > 0 && n.is_none_or(|n| n == m.rows()) =>
            {
                Some((name.to_owned(), m.rows()))
            }
            _ => None,
        })
        .collect()
}

fn handle_picker(ui: &mut egui::Ui, id: &str, text: &mut String, handles: &[String]) {
    if handles.is_empty() {
        return;
    }
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text("Workspace handles")
        .show_ui(ui, |ui| {
            for name in handles {
                if ui.selectable_label(text == name, name).clicked() {
                    *text = name.clone();
                }
            }
        });
}

fn parameters_ui(ui: &mut egui::Ui, spec: &mut Spec, ws: &Snapshot) {
    let p = crate::tokens::palette(ui.ctx());
    ui.label("Parameters");
    let width = ui.available_width();
    let mut remove = None;
    egui::Grid::new("solve_ode_parameters")
        .num_columns(4)
        .spacing(egui::vec2(crate::tokens::XS, crate::tokens::XS))
        .show(ui, |ui| {
            for (i, (name, value)) in spec.parameters.iter_mut().enumerate() {
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(name).char_limit(63))
                        .hint_text("name")
                        .desired_width(72.),
                );
                ui.label("=");
                ui.add(
                    crate::tokens::input(egui::TextEdit::singleline(value).char_limit(1024))
                        .hint_text("value")
                        .desired_width((width - 72. - 64.).max(60.)),
                );
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        spec.parameters.remove(i);
    }
    if spec.parameters.len() < MAX_PARAMETERS && ui.button("Add parameter").clicked() {
        spec.parameters.push((String::new(), String::new()));
    }
    for (name, _) in &spec.parameters {
        let name = name.trim();
        if !name.is_empty() && ws.contains(name) {
            ui.label(
                egui::RichText::new(format!("Export replaces the workspace variable {name}"))
                    .size(crate::tokens::CAPTION)
                    .color(p.warning),
            );
        }
    }
}

fn events_ui(ui: &mut egui::Ui, spec: &mut Spec, handles: &[String]) {
    let p = crate::tokens::palette(ui.ctx());
    let width = ui.available_width();
    field(ui, "Events", |ui| {
        ui.horizontal(|ui| {
            for events in Events::ALL {
                if crate::tokens::toggle(ui, spec.events == events, events.label()).clicked() {
                    spec.events = events;
                }
            }
        });
    });
    match spec.events {
        Events::None => return,
        Events::Condition => {
            field(ui, "Event when this crosses zero", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut spec.event_value).char_limit(4096),
                    )
                    .hint_text("y1")
                    .desired_width(width),
                );
            });
            field(ui, "Direction", |ui| {
                ui.horizontal(|ui| {
                    for direction in Direction::ALL {
                        if crate::tokens::toggle(ui, spec.direction == direction, direction.label())
                            .clicked()
                        {
                            spec.direction = direction;
                        }
                    }
                });
            });
            ui.checkbox(&mut spec.terminal, "Stop at the first event");
        }
        Events::Function => {
            field(ui, "Event function", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut spec.event_function).char_limit(4096),
                    )
                    .hint_text("name or handle variable")
                    .desired_width(width),
                );
                handle_picker(
                    ui,
                    "solve_ode_event_handles",
                    &mut spec.event_function,
                    handles,
                );
                crate::tokens::helper(ui, "[value, isterminal, direction] = events(t, y)");
            });
        }
    }
    let solver = spec.resolved();
    if !solver.events() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "{} has no event location in MatRust yet: choose Auto, ode113 or a stiff solver.",
                    solver.name()
                ))
                .size(crate::tokens::CAPTION)
                .color(p.warning),
            )
            .wrap(),
        );
    }
}

// ---------------------------------------------------------------------------
// Plots

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
    let [r, g, b] = matrust::plot::COLORS[k % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}
/// Finite points, every k-th one past MAX_POINTS (the last always).
fn thin(x: &[f64], y: &[f64]) -> Vec<[f64; 2]> {
    let step = x.len().div_ceil(MAX_POINTS).max(1);
    x.iter()
        .zip(y)
        .enumerate()
        .filter(|(i, _)| i % step == 0 || *i + 1 == x.len())
        .map(|(_, (x, y))| [*x, *y])
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .collect()
}

fn time_plot(ui: &mut egui::Ui, height: f32, r: &Solution) {
    let p = crate::tokens::palette(ui.ctx());
    chart("solve_ode_time", height, ("t", "y"))
        .legend(egui_plot::Legend::default().follow_insertion_order(true))
        .show(ui, |plot_ui| {
            for k in 0..r.n {
                plot_ui.line(
                    egui_plot::Line::new(r.labels[k].clone(), thin(&r.t, r.component(k)))
                        .color(color(k))
                        .width(1.5),
                );
            }
            if r.events() > 0 {
                let points: Vec<[f64; 2]> = (0..r.events())
                    .flat_map(|j| (0..r.n).map(move |k| [r.te[j], r.event(j, k)]))
                    .collect();
                plot_ui.points(
                    egui_plot::Points::new("Events", points)
                        .shape(egui_plot::MarkerShape::Circle)
                        .filled(false)
                        .radius(4.)
                        .color(p.text),
                );
            }
        });
}

fn phase_plot(ui: &mut egui::Ui, height: f32, r: &Solution, [i, j]: [usize; 2]) {
    let p = crate::tokens::palette(ui.ctx());
    if i >= r.n || j >= r.n {
        crate::tokens::helper(ui, "The phase plane needs two components.");
        return;
    }
    chart(
        "solve_ode_phase",
        height,
        (r.labels[i].as_str(), r.labels[j].as_str()),
    )
    .show(ui, |plot_ui| {
        plot_ui.line(
            egui_plot::Line::new("Trajectory", thin(r.component(i), r.component(j)))
                .color(color(0))
                .width(1.5),
        );
        if r.events() > 0 {
            let points: Vec<[f64; 2]> = (0..r.events())
                .map(|e| [r.event(e, i), r.event(e, j)])
                .collect();
            plot_ui.points(
                egui_plot::Points::new("Events", points)
                    .shape(egui_plot::MarkerShape::Circle)
                    .filled(false)
                    .radius(4.)
                    .color(p.text),
            );
        }
    });
}

fn range(values: &[f64]) -> (f64, f64) {
    let (lo, hi) = values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    if lo > hi {
        (-1., 1.)
    } else if lo == hi {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    }
}

/// A 3-D trajectory in a box, turned by dragging.
fn phase3_plot(
    ui: &mut egui::Ui,
    height: f32,
    r: &Solution,
    components: [usize; 3],
    angles: &mut (f32, f32),
) {
    let p = crate::tokens::palette(ui.ctx());
    if components.iter().any(|c| *c >= r.n) {
        crate::tokens::helper(ui, "The 3-D phase plot needs three components.");
        return;
    }
    crate::tokens::helper(ui, "Drag to turn the box.");
    let size = egui::vec2(
        ui.available_width(),
        (height - crate::tokens::ROW).max(160.),
    );
    let (response, painter) = ui.allocate_painter(size, egui::Sense::drag());
    if response.dragged() {
        let d = response.drag_delta();
        angles.0 -= d.x * 0.01;
        angles.1 = (angles.1 + d.y * 0.01).clamp(-1.5, 1.5);
    }
    let rect = response.rect;
    let ranges = components.map(|c| range(r.component(c)));
    let (sa, ca) = (angles.0 as f64).sin_cos();
    let (se, ce) = (angles.1 as f64).sin_cos();
    let scale = 0.32 * rect.width().min(rect.height()) as f64;
    let center = rect.center();
    let project = |q: [f64; 3]| {
        let (x, y, z) = (q[0], q[1], q[2]);
        let across = ca * x - sa * y;
        let depth = sa * x + ca * y;
        let up = ce * z - se * depth;
        egui::pos2(
            center.x + (across * scale) as f32,
            center.y - (up * scale) as f32,
        )
    };
    let normal = |v: f64, (lo, hi): (f64, f64)| 2. * (v - lo) / (hi - lo) - 1.;
    let edge = egui::Stroke::new(1., p.border);
    for a in 0..8u8 {
        for bit in 0..3 {
            let b = a | (1 << bit);
            if b == a {
                continue;
            }
            let corner =
                |c: u8| [0u8, 1, 2].map(|axis| if c & (1 << axis) != 0 { 1. } else { -1. });
            painter.line_segment([project(corner(a)), project(corner(b))], edge);
        }
    }
    let font = egui::FontId::proportional(crate::tokens::CAPTION);
    for (axis, end) in [[1.15, -1., -1.], [-1., 1.15, -1.], [-1., -1., 1.15]]
        .into_iter()
        .enumerate()
    {
        painter.text(
            project(end),
            egui::Align2::CENTER_CENTER,
            &r.labels[components[axis]],
            font.clone(),
            p.text_muted,
        );
    }
    let [x, y, z] = components.map(|c| r.component(c));
    let step = r.points().div_ceil(MAX_POINTS).max(1);
    let points: Vec<egui::Pos2> = (0..r.points())
        .filter(|i| i % step == 0 || *i + 1 == r.points())
        .map(|i| {
            [
                normal(x[i], ranges[0]),
                normal(y[i], ranges[1]),
                normal(z[i], ranges[2]),
            ]
        })
        .filter(|q| q.iter().all(|v| v.is_finite()))
        .map(project)
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color(0))));
    for e in 0..r.events() {
        let q = [0usize, 1, 2].map(|axis| normal(r.event(e, components[axis]), ranges[axis]));
        if q.iter().all(|v| v.is_finite()) {
            painter.circle_stroke(project(q), 4., egui::Stroke::new(1.5, p.text));
        }
    }
}
