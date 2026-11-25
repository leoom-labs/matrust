//! Spline construction is engine code: one plan supplies preview, export and save.
//! Public function contracts: MathWorks csapi/csape/csaps/spaps/spap2/fnder/fnplt.
use super::{App, AppKind, Outputs, Snapshot, field, name_field, number, short, unused_name};
use eframe::egui;
use matrust::{Array, Matrix};
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

pub const MAX_SPLINES: usize = 5;
pub const MAX_SITES: usize = 10_000;
pub const MAX_COMPONENTS: usize = 8;
pub const MAX_GRID: usize = 4096;
pub const MAX_KNOTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Cubic,
    Smooth,
    Tolerance,
    LeastSquares,
    Pchip,
}
impl Method {
    pub const ALL: [Self; 5] = [
        Self::Cubic,
        Self::Smooth,
        Self::Tolerance,
        Self::LeastSquares,
        Self::Pchip,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cubic => "Interpolating cubic",
            Self::Smooth => "Smoothing · p",
            Self::Tolerance => "Smoothing · tolerance",
            Self::LeastSquares => "Least squares",
            Self::Pchip => "Shape preserving",
        }
    }
    fn weighted(self) -> bool {
        matches!(self, Self::Smooth | Self::Tolerance | Self::LeastSquares)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ends {
    NotAKnot,
    Natural,
    Clamped,
    Second,
    Periodic,
}
impl Ends {
    pub const ALL: [Self; 5] = [
        Self::NotAKnot,
        Self::Natural,
        Self::Clamped,
        Self::Second,
        Self::Periodic,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::NotAKnot => "Not-a-knot",
            Self::Natural => "Natural",
            Self::Clamped => "End slopes",
            Self::Second => "End curvatures",
            Self::Periodic => "Periodic",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Fit {
    pub id: u32,
    pub x: String,
    pub y: String,
    pub weights: String,
    pub method: Method,
    pub ends: Ends,
    pub left: String,
    pub right: String,
    pub p: String,
    pub tolerance: String,
    /// Strictly interior, simple knots. The script adds clamped endpoints.
    pub knots: String,
    pub order: String,
    pub grid_start: String,
    pub grid_end: String,
    pub grid_count: usize,
    pub spline_name: String,
    pub values_name: String,
}
impl Fit {
    fn new(id: u32) -> Self {
        Self {
            id,
            x: "x".into(),
            y: "y".into(),
            weights: String::new(),
            method: Method::Cubic,
            ends: Ends::NotAKnot,
            left: "0".into(),
            right: "0".into(),
            p: "0.9".into(),
            tolerance: "0.01".into(),
            knots: "[]".into(),
            order: "4".into(),
            grid_start: String::new(),
            grid_end: String::new(),
            grid_count: 401,
            spline_name: format!("spline{id}"),
            values_name: format!("spline_values{id}"),
        }
    }
    fn settings(&self) -> String {
        match self.method {
            Method::Cubic => match self.ends {
                Ends::Clamped | Ends::Second => {
                    format!("{} [{}, {}]", self.ends.label(), self.left, self.right)
                }
                _ => self.ends.label().into(),
            },
            Method::Smooth => format!("p = {}", self.p),
            Method::Tolerance => format!("tolerance = {}", self.tolerance),
            Method::LeastSquares => format!("order {} · interior {}", self.order, self.knots),
            Method::Pchip => "pchip".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Curve {
    pub id: u32,
    pub method: Method,
    pub settings: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub grid: Vec<f64>,
    pub values: Vec<f64>,
    pub first: Vec<f64>,
    pub second: Vec<f64>,
    pub residuals: Vec<f64>,
    pub knots: Vec<f64>,
    pub components: usize,
    /// Frobenius residual norm, integral of sum of squared second derivatives.
    pub metrics: [f64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Spline,
    First,
    Second,
    Residuals,
}
impl View {
    const ALL: [Self; 4] = [Self::Spline, Self::First, Self::Second, Self::Residuals];
    fn label(self) -> &'static str {
        match self {
            Self::Spline => "Spline",
            Self::First => "First derivative",
            Self::Second => "Second derivative",
            Self::Residuals => "Residuals",
        }
    }
}
pub struct SplineTool {
    pub fits: Vec<Fit>,
    pub selected: usize,
    pub current: Vec<Curve>,
    pub view: View,
    pub running: bool,
    next_id: u32,
    stopped: Option<u64>,
    drag: Option<usize>,
    knot_error: Option<String>,
    knot_to_add: String,
    range: Option<(f64, f64)>,
    settings_key: Option<u64>,
}
impl Default for SplineTool {
    fn default() -> Self {
        Self {
            fits: vec![Fit::new(1)],
            selected: 0,
            current: Vec::new(),
            view: View::Spline,
            running: false,
            next_id: 2,
            stopped: None,
            drag: None,
            knot_error: None,
            knot_to_add: "0.5".into(),
            range: None,
            settings_key: None,
        }
    }
}
const PARTS: [&str; 9] = [
    "x",
    "y",
    "grid",
    "values",
    "first",
    "second",
    "residuals",
    "knots",
    "metrics",
];
struct Record {
    vars: [String; 9],
}
struct Plan {
    script: String,
    records: Vec<Record>,
}

fn scalar(text: &str, label: &str) -> Result<f64, String> {
    if text.len() > 64 {
        return Err(format!("Spline Tool: {label}: at most 64 characters"));
    }
    super::single_expression(text)?;
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("Spline Tool: {label}: enter one finite numeric literal"))
}
pub fn parse_knots(text: &str) -> Result<Vec<f64>, String> {
    if text.len() > 2048 {
        return Err("Spline Tool: knot expression is too long".into());
    }
    super::single_expression(text)?;
    let inner = text
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("Spline Tool: use a bracketed vector of numeric knot literals")?;
    let knots = inner
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| scalar(s, "Knot"))
        .collect::<Result<Vec<_>, _>>()?;
    if knots.len() > MAX_KNOTS || knots.windows(2).any(|w| w[0] >= w[1]) {
        return Err(format!(
            "Spline Tool: use up to {MAX_KNOTS} strictly increasing interior knots"
        ));
    }
    Ok(knots)
}
fn knot_text(knots: &[f64]) -> String {
    format!(
        "[{}]",
        knots
            .iter()
            .map(|v| number(*v))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn numeric<'a>(ws: &'a Snapshot, name: &str) -> Result<&'a Matrix, String> {
    if name.len() > 63 {
        return Err("Spline Tool: workspace name is too long".into());
    }
    super::single_expression(name)?;
    if !matrust::files::valid_name(name) {
        return Err("Spline Tool: choose a workspace variable".into());
    }
    match ws.get(name) {
        Some(Array::Numeric(m))
            if !m.is_empty()
                && m.len() <= MAX_SITES * MAX_COMPONENTS
                && m.dims().len() <= 2
                && m.data().iter().all(|v| v.is_finite()) =>
        {
            Ok(m)
        }
        _ => Err(format!(
            "Spline Tool: {name} must be a finite real numeric array"
        )),
    }
}
fn source_range(f: &Fit, ws: &Snapshot) -> Result<(f64, f64), String> {
    let x = numeric(ws, &f.x)?;
    if (x.rows() != 1 && x.cols() != 1)
        || !(2..=MAX_SITES).contains(&x.len())
        || x.data().windows(2).any(|w| w[0] >= w[1])
    {
        return Err(format!(
            "Spline Tool: x must be a strictly increasing vector of 2–{MAX_SITES} sites"
        ));
    }
    let y = numeric(ws, &f.y)?;
    let valid_y = if y.rows() == 1 || y.cols() == 1 {
        y.len() == x.len()
    } else {
        y.cols() == x.len() && y.rows() <= MAX_COMPONENTS
    };
    if !valid_y || y.dims().len() > 2 {
        return Err(format!(
            "Spline Tool: y must have one column per site and up to {MAX_COMPONENTS} rows (a vector is also accepted)"
        ));
    }
    if f.method.weighted() && !f.weights.is_empty() {
        let w = numeric(ws, &f.weights)?;
        if (w.rows() != 1 && w.cols() != 1)
            || w.len() != x.len()
            || w.data().iter().any(|v| *v < 0.)
            || w.data().iter().filter(|v| **v > 0.).count() < 2
        {
            return Err("Spline Tool: weights need one nonnegative value per site, with at least two positive values".into());
        }
    }
    Ok((x.data()[0], x.data()[x.len() - 1]))
}
impl SplineTool {
    pub fn add(&mut self, ws: &Snapshot) -> Result<(), String> {
        if self.fits.len() >= MAX_SPLINES {
            return Err("Spline Tool: at most five splines".into());
        }
        let mut fit = self
            .fits
            .get(self.selected)
            .cloned()
            .unwrap_or_else(|| Fit::new(self.next_id));
        fit.id = self.next_id;
        self.next_id += 1;
        fit.spline_name = unused_name("spline", |n| {
            ws.contains(n)
                || self
                    .fits
                    .iter()
                    .any(|f| f.spline_name == n || f.values_name == n)
        });
        fit.values_name = unused_name("spline_values", |n| {
            ws.contains(n)
                || n == fit.spline_name
                || self
                    .fits
                    .iter()
                    .any(|f| f.spline_name == n || f.values_name == n)
        });
        self.fits.push(fit);
        self.selected = self.fits.len() - 1;
        self.drag = None;
        Ok(())
    }
    pub fn remove(&mut self) {
        if self.selected < self.fits.len() {
            self.fits.remove(self.selected);
        }
        self.selected = self.selected.min(self.fits.len().saturating_sub(1));
        self.drag = None;
    }
    pub fn move_knot(&mut self, index: usize, value: f64, range: (f64, f64)) -> Result<(), String> {
        if !value.is_finite() || value <= range.0 || value >= range.1 {
            return Err("Spline Tool: a knot must stay strictly inside the data range".into());
        }
        let fit = self
            .fits
            .get_mut(self.selected)
            .ok_or("Spline Tool: select a spline")?;
        let mut knots = parse_knots(&fit.knots)?;
        if index >= knots.len() {
            return Err("Spline Tool: knot no longer exists".into());
        }
        if knots
            .iter()
            .enumerate()
            .any(|(i, k)| i != index && *k == value)
        {
            return Err("Spline Tool: knots must be distinct".into());
        }
        knots[index] = value;
        knots.sort_by(f64::total_cmp);
        self.drag = knots.iter().position(|k| *k == value);
        fit.knots = knot_text(&knots);
        Ok(())
    }
    pub fn add_knot(&mut self, value: f64, range: (f64, f64)) -> Result<(), String> {
        if !value.is_finite() || value <= range.0 || value >= range.1 {
            return Err("Spline Tool: a knot must stay strictly inside the data range".into());
        }
        let fit = self
            .fits
            .get_mut(self.selected)
            .ok_or("Spline Tool: select a spline")?;
        let mut knots = parse_knots(&fit.knots)?;
        if knots.len() == MAX_KNOTS || knots.contains(&value) {
            return Err("Spline Tool: knot limit reached or duplicate knot".into());
        }
        knots.push(value);
        knots.sort_by(f64::total_cmp);
        fit.knots = knot_text(&knots);
        Ok(())
    }
    pub fn remove_knot(&mut self, index: usize) -> Result<(), String> {
        let fit = self
            .fits
            .get_mut(self.selected)
            .ok_or("Spline Tool: select a spline")?;
        let mut knots = parse_knots(&fit.knots)?;
        if index >= knots.len() {
            return Err("Spline Tool: knot no longer exists".into());
        }
        knots.remove(index);
        fit.knots = knot_text(&knots);
        self.drag = None;
        Ok(())
    }
    fn key(&self, ws: &Snapshot, script: &str) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        script.hash(&mut h);
        for name in self.inputs() {
            name.hash(&mut h);
            ws.version(&name).hash(&mut h);
        }
        h.finish()
    }
    pub fn stop(&mut self, ws: &Snapshot) {
        if let Ok(plan) = self.plan(ws) {
            self.stopped = Some(self.key(ws, &plan.script));
        }
    }
    pub fn run(&mut self) {
        self.stopped = None;
        self.knot_error = None;
    }
    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        if self.fits.is_empty() || self.fits.len() > MAX_SPLINES {
            return Err("Spline Tool: keep one to five splines".into());
        }
        if let Some(error) = &self.knot_error {
            return Err(error.clone());
        }
        let mut names = BTreeSet::new();
        for f in &self.fits {
            for name in [&f.spline_name, &f.values_name] {
                if name.len() > 63 || !matrust::files::valid_name(name) {
                    return Err("Spline Tool: export names must be valid variable names of at most 63 characters".into());
                }
                super::single_expression(name)?;
                matrust::syntax::parse(&format!("{name} = 0;"))
                    .map_err(|_| "Spline Tool: export names cannot be language keywords")?;
                super::check_export_name(name)?;
                if !names.insert(name.clone()) {
                    return Err("Spline Tool: export names must be different".into());
                }
            }
        }
        for f in &self.fits {
            for part in PARTS {
                names.insert(format!("st_probe_{}_{part}", f.id));
            }
        }
        let mut fresh = |base: &str| {
            let n = unused_name(base, |n| ws.contains(n) || names.contains(n));
            names.insert(n.clone());
            n
        };
        let helper = fresh("st_roughness");
        let pchip_helper = fresh("st_pchip");
        let mut script = String::from(
            "% Spline Tool: y has one column per x site; vectors are one component.\n% Roughness = integral of the sum of squared second derivatives.\n",
        );
        let mut records = Vec::new();
        let mut exports = String::new();
        for f in &self.fits {
            let range = source_range(f, ws)?;
            let vars = std::array::from_fn(|i| fresh(&format!("st_{}", PARTS[i])));
            let [x, y, grid, values, first, second, residuals, knots, metrics] = &vars;
            let spline = fresh("st_spline");
            let w = fresh("st_weights");
            script.push_str(&format!(
                "{x} = {}(:)';\n{y} = {};\nif isvector({y}), {y} = reshape({y},1,[]); end\n",
                f.x, f.y
            ));
            script.push_str(&format!("if numel({x}) < 2 || numel({x}) > {MAX_SITES} || any(~isfinite({x})) || any(diff({x}) <= 0), error('Spline Tool: x must be a finite increasing vector of 2 to {MAX_SITES} sites'); end\nif ~isnumeric({y}) || ~isreal({y}) || size({y},2) ~= numel({x}) || size({y},1) > {MAX_COMPONENTS} || any(~isfinite({y}(:))), error('Spline Tool: y must have one column per site and at most {MAX_COMPONENTS} finite real rows'); end\n"));
            let weight = if f.method.weighted() && !f.weights.is_empty() {
                script.push_str(&format!("{w} = {}(:)';\nif numel({w}) ~= numel({x}) || any(~isfinite({w})) || any({w} < 0) || sum({w} > 0) < 2, error('Spline Tool: invalid weights'); end\n", f.weights));
                w.as_str()
            } else {
                "[]"
            };
            let call = match f.method {
                Method::Cubic => match f.ends {
                    Ends::NotAKnot => format!("csapi({x}, {y})"),
                    Ends::Natural => format!("csape({x}, {y}, 'variational')"),
                    Ends::Periodic => format!("csape({x}, {y}, 'periodic')"),
                    Ends::Clamped | Ends::Second => {
                        let l = scalar(&f.left, "Left end value")?;
                        let r = scalar(&f.right, "Right end value")?;
                        let cond = if f.ends == Ends::Clamped {
                            "complete"
                        } else {
                            "second"
                        };
                        // Public three-argument csape syntax, one value for every component.
                        format!(
                            "csape({x}, [{}*ones(size({y},1),1) {y} {}*ones(size({y},1),1)], '{cond}')",
                            number(l),
                            number(r)
                        )
                    }
                },
                Method::Smooth => {
                    let p = scalar(&f.p, "p")?;
                    if !(0. ..=1.).contains(&p) {
                        return Err("Spline Tool: p must be between 0 and 1".into());
                    }
                    format!("csaps({x}, {y}, {}, [], {weight})", number(p))
                }
                Method::Tolerance => {
                    let t = scalar(&f.tolerance, "Tolerance")?;
                    if t < 0. {
                        return Err("Spline Tool: tolerance must be nonnegative".into());
                    }
                    format!("spaps({x}, {y}, {}, {weight})", number(t))
                }
                Method::LeastSquares => {
                    let k = scalar(&f.order, "Order")?;
                    if !(2. ..=8.).contains(&k) || k.fract() != 0. {
                        return Err("Spline Tool: order must be an integer from 2 to 8".into());
                    }
                    let interior = parse_knots(&f.knots)?;
                    if interior.iter().any(|k| *k <= range.0 || *k >= range.1) {
                        return Err(
                            "Spline Tool: knots must be strictly inside the data range".into()
                        );
                    }
                    let t = fresh("st_knots");
                    script.push_str(&format!("{t} = {};\nif any({t} <= {x}(1)) || any({t} >= {x}(end)), error('Spline Tool: interior knot outside the data range'); end\n{t} = augknt([{x}(1) {t} {x}(end)], {});\n", knot_text(&interior), number(k)));
                    let opt = if weight == "[]" {
                        String::new()
                    } else {
                        format!(", {weight}")
                    };
                    format!("spap2({t}, {}, {x}, {y}{opt})", number(k))
                }
                Method::Pchip => format!("{pchip_helper}({x}, {y})"),
            };
            let lo = if f.grid_start.is_empty() {
                format!("{x}(1)")
            } else {
                number(scalar(&f.grid_start, "Grid start")?)
            };
            let hi = if f.grid_end.is_empty() {
                format!("{x}(end)")
            } else {
                number(scalar(&f.grid_end, "Grid end")?)
            };
            if !(2..=MAX_GRID).contains(&f.grid_count) {
                return Err(format!("Spline Tool: grid needs 2 to {MAX_GRID} points"));
            }
            if !f.grid_start.is_empty()
                && !f.grid_end.is_empty()
                && scalar(&f.grid_start, "Grid start")? >= scalar(&f.grid_end, "Grid end")?
            {
                return Err("Spline Tool: grid start must precede its end".into());
            }
            script.push_str(&format!("{spline} = {call};\n{grid} = linspace({lo}, {hi}, {});\nif {grid}(1) >= {grid}(end) || {grid}(1) < {x}(1) || {grid}(end) > {x}(end), error('Spline Tool: grid must increase inside the data range'); end\n{values} = fnval({spline}, {grid});\n{first} = fnval(fnder({spline},1), {grid});\n{second} = fnval(fnder({spline},2), {grid});\n{residuals} = {y} - fnval({spline}, {x});\n", f.grid_count));
            let part = if matches!(f.method, Method::Tolerance | Method::LeastSquares) {
                "knots"
            } else {
                "breaks"
            };
            script.push_str(&format!("{knots} = fnbrk({spline}, '{part}');\n{metrics} = [norm({residuals},'fro') {helper}({spline},unique({knots}))];\n"));
            // The app's Spline view: every component against the x grid.
            script.push_str(&format!(
                "figure; plot({grid}, {values}'); xlabel('x'); ylabel('Spline');\n"
            ));
            exports.push_str(&format!(
                "{} = {spline};\n{} = {values};\n",
                f.spline_name, f.values_name
            ));
            records.push(Record { vars });
        }
        script.push_str(&exports);
        script.push_str(&ROUGHNESS.replace("ST_ROUGHNESS", &helper));
        script.push_str(&PCHIP_ROWS.replace("ST_PCHIP", &pchip_helper));
        Ok(Plan { script, records })
    }
}
// Eight-point Gauss-Legendre quadrature on every polynomial piece. Exact for
// squared second derivatives through order 8 (degree <= 10); no numeric kernel
// is reimplemented by the UI. fnder/fnval and reductions run in the Runner.
const ROUGHNESS: &str = "\nfunction r = ST_ROUGHNESS(f,b)\n\
    a = (b(1:end-1)+b(2:end))/2; h = diff(b)/2;\n\
    z = [-0.9602898564975363 -0.7966664774136267 -0.5255324099163290 -0.1834346424956498 0.1834346424956498 0.5255324099163290 0.7966664774136267 0.9602898564975363];\n\
    w = [0.1012285362903763 0.2223810344533745 0.3137066458778873 0.3626837833783620 0.3626837833783620 0.3137066458778873 0.2223810344533745 0.1012285362903763];\n\
    r = 0; d = fnder(f,2);\n\
    for j = 1:8\n\
        v = fnval(d,a+h*z(j));\n\
        r = r + w(j)*sum(sum(v.^2,1).*h);\n\
    end\n\
end\n";

// The engine currently accepts vector y for pchip. Assemble the public pp
// representation from scalar pchip calls, preserving piece/component row order.
const PCHIP_ROWS: &str = "\nfunction pp = ST_PCHIP(x,y)\n\
    d = size(y,1);\n\
    pp = pchip(x,y(1,:));\n\
    if d > 1\n\
        c = zeros(d*fnbrk(pp,'pieces'),fnbrk(pp,'order'));\n\
        c(1:d:end,:) = fnbrk(pp,'coefs');\n\
        for j = 2:d\n\
            p = pchip(x,y(j,:));\n\
            c(j:d:end,:) = fnbrk(p,'coefs');\n\
        end\n\
        pp = mkpp(fnbrk(pp,'breaks'),c,d);\n\
    end\n\
end\n";

impl App for SplineTool {
    fn kind(&self) -> AppKind {
        AppKind::SplineTool
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.plan(ws)?;
        if self.stopped == Some(self.key(ws, &plan.script)) {
            return Err("Spline Tool: stopped; choose Run to resume".into());
        }
        Ok(plan.script)
    }
    fn inputs(&self) -> Vec<String> {
        self.fits
            .iter()
            .flat_map(|f| {
                [&f.x, &f.y]
                    .into_iter()
                    .chain((f.method.weighted() && !f.weights.is_empty()).then_some(&f.weights))
            })
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        let Ok(plan) = self.plan(ws) else {
            return (String::new(), Vec::new());
        };
        let mut code = String::new();
        let mut names = Vec::new();
        for (f, r) in self.fits.iter().zip(plan.records) {
            for (part, source) in PARTS.iter().zip(r.vars) {
                let name = format!("st_probe_{}_{part}", f.id);
                code.push_str(&format!("{name} = {source};\n"));
                names.push(name);
            }
        }
        (code, names)
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let mut next = Vec::new();
        for f in &self.fits {
            let read = |part: &str| -> Result<Vec<f64>, String> {
                match outputs.get(&format!("st_probe_{}_{part}", f.id)) {
                    Some(Array::Numeric(m))
                        if m.len() <= MAX_SITES * MAX_COMPONENTS
                            && m.data().iter().all(|v| v.is_finite()) =>
                    {
                        Ok(m.data().to_vec())
                    }
                    _ => Err(format!("Spline Tool: invalid or missing {part} result")),
                }
            };
            let x = read("x")?;
            let y = read("y")?;
            let grid = read("grid")?;
            let values = read("values")?;
            let first = read("first")?;
            let second = read("second")?;
            let residuals = read("residuals")?;
            let knots = read("knots")?;
            let metrics = read("metrics")?;
            if x.len() < 2
                || x.len() > MAX_SITES
                || !(2..=MAX_GRID).contains(&grid.len())
                || !y.len().is_multiple_of(x.len())
                || metrics.len() != 2
            {
                return Err("Spline Tool: incomplete result".into());
            }
            let components = y.len() / x.len();
            if !(1..=MAX_COMPONENTS).contains(&components)
                || values.len() != grid.len() * components
                || first.len() != values.len()
                || second.len() != values.len()
                || residuals.len() != y.len()
                || knots.len() < 2
            {
                return Err("Spline Tool: inconsistent result dimensions".into());
            }
            next.push(Curve {
                id: f.id,
                method: f.method,
                settings: f.settings(),
                x,
                y,
                grid,
                values,
                first,
                second,
                residuals,
                knots,
                components,
                metrics: [metrics[0], metrics[1]],
            });
        }
        self.current = next;
        Ok(())
    }
    fn prepare(&mut self, ws: &Snapshot) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.fits).hash(&mut h);
        for name in self.inputs() {
            ws.version(&name).hash(&mut h);
        }
        let key = h.finish();
        if self.settings_key != Some(key) {
            self.knot_error = None;
            self.stopped = None;
            self.settings_key = Some(key);
        }
        self.selected = self.selected.min(self.fits.len().saturating_sub(1));
        self.range = self
            .fits
            .get(self.selected)
            .and_then(|f| source_range(f, ws).ok());
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.horizontal(|ui| {
            let keep = ui.add_enabled(
                self.fits.len() < MAX_SPLINES,
                egui::Button::new("Keep a copy"),
            );
            crate::focus_areas::register(
                ui,
                &keep,
                crate::focus_areas::Area::App(AppKind::SplineTool),
            );
            if keep.clicked() {
                self.knot_error = self.add(ws).err();
            }
            if ui
                .add_enabled(!self.fits.is_empty(), egui::Button::new("Remove"))
                .clicked()
            {
                self.remove();
            }
        });
        if (self.stopped.is_some() || self.knot_error.is_some()) && ui.button("Run").clicked() {
            self.run();
        }
        if self.running && ui.button("Stop").clicked() {
            self.stop(ws);
        }
        let Some(f) = self.fits.get_mut(self.selected) else {
            return;
        };
        variable_ui(ui, "x sites", &mut f.x, ws, false, true);
        variable_ui(
            ui,
            "y values (rows = components)",
            &mut f.y,
            ws,
            false,
            false,
        );
        field(ui, "Method", |ui| {
            egui::ComboBox::from_id_salt("st_method")
                .selected_text(f.method.label())
                .show_ui(ui, |ui| {
                    for method in Method::ALL {
                        ui.selectable_value(&mut f.method, method, method.label());
                    }
                });
        });
        if f.method.weighted() {
            variable_ui(ui, "Weights (optional)", &mut f.weights, ws, true, true);
        }
        match f.method {
            Method::Cubic => {
                field(ui, "End conditions", |ui| {
                    egui::ComboBox::from_id_salt("st_ends")
                        .selected_text(f.ends.label())
                        .show_ui(ui, |ui| {
                            for ends in Ends::ALL {
                                ui.selectable_value(&mut f.ends, ends, ends.label());
                            }
                        });
                });
                if matches!(f.ends, Ends::Clamped | Ends::Second) {
                    text_field(ui, "Left end value (all rows)", &mut f.left, 64);
                    text_field(ui, "Right end value (all rows)", &mut f.right, 64);
                }
            }
            Method::Smooth => {
                text_field(ui, "p (0 = line, 1 = natural interpolant)", &mut f.p, 64);
            }
            Method::Tolerance => {
                text_field(ui, "Weighted squared error tolerance", &mut f.tolerance, 64);
            }
            Method::LeastSquares => {
                text_field(ui, "Order (2–8)", &mut f.order, 64);
                if text_field(ui, "Interior knots [numeric literals]", &mut f.knots, 2048) {
                    self.knot_error = None;
                }
            }
            Method::Pchip => {}
        }
        if f.method == Method::LeastSquares {
            crate::tokens::helper(
                ui,
                "Drag the knot ticks in the spline plot. Endpoints stay fixed.",
            );
            let knots = parse_knots(&f.knots).unwrap_or_default();
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (i, k) in knots.iter().enumerate() {
                    if ui
                        .small_button(format!("× {}", short(*k)))
                        .on_hover_text("Remove knot")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                }
            });
            if let Some(i) = remove {
                self.knot_error = self.remove_knot(i).err();
            }
            text_field(ui, "New interior knot", &mut self.knot_to_add, 64);
            if ui.button("Add knot").clicked() {
                self.knot_error = scalar(&self.knot_to_add, "New knot")
                    .and_then(|v| {
                        self.add_knot(v, self.range.ok_or("Spline Tool: choose data first")?)
                    })
                    .err();
            }
        }
        crate::tokens::helper(
            ui,
            "Limits: 10,000 sites, 8 rows, 5 splines. Numeric settings accept literals.",
        );
    }
    fn main_caption(&self) -> &'static str {
        "SPLINE"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for view in View::ALL {
                if crate::tokens::tab(ui, self.view == view, view.label()).clicked() {
                    self.view = view;
                    self.drag = None;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let Some(f) = self.fits.get(self.selected) else {
            crate::tokens::helper(ui, "Choose data in INPUTS.");
            return;
        };
        let Some(c) = self.current.iter().find(|c| c.id == f.id) else {
            crate::tokens::helper(
                ui,
                "Choose x and y in INPUTS. The status line shows what is missing.",
            );
            return;
        };
        let derivative = matches!(self.view, View::First | View::Second);
        let main_view = if derivative { View::Spline } else { self.view };
        let editable = f.method == Method::LeastSquares
            && c.method == Method::LeastSquares
            && main_view == View::Spline;
        let edit_knots = if editable {
            parse_knots(&f.knots).ok().filter(|knots| {
                self.range
                    .is_some_and(|(a, b)| knots.iter().all(|k| *k > a && *k < b))
            })
        } else {
            None
        };
        let mut ticks = edit_knots.clone().unwrap_or_else(|| c.knots.clone());
        if edit_knots.is_some()
            && let Some((a, b)) = self.range
        {
            ticks.extend([a, b]);
        }
        ticks.sort_by(f64::total_cmp);
        ticks.dedup();
        let height = ui.available_height().max(240.);
        let chart_height = if derivative {
            (height - crate::tokens::S) / 2.
        } else {
            height
        };
        let response = draw_curve(ui, c, main_view, chart_height, &ticks, "main");
        if derivative {
            ui.add_space(crate::tokens::S);
            draw_curve(ui, c, self.view, chart_height, &ticks, "derivative");
        }
        // PlotUi::pointer_coordinate compensates for panning even when dragging
        // is disabled. Use the final transform and the actual screen pointer.
        let pointer = ui
            .input(|i| i.pointer.latest_pos())
            .map(|pos| response.transform.value_from_position(pos));
        if editable {
            if response.response.drag_started()
                && let Some(at) = pointer
            {
                self.drag = edit_knots
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| (*a - at.x).abs().total_cmp(&(*b - at.x).abs()))
                    .and_then(|(i, k)| {
                        let px = response
                            .transform
                            .position_from_point(&egui_plot::PlotPoint::new(*k, at.y));
                        let ptr = response.transform.position_from_point(&at);
                        ((px.x - ptr.x).abs() <= 12.).then_some(i)
                    });
            }
            if response.response.dragged()
                && let (Some(i), Some(at), Some(range)) = (self.drag, pointer, self.range)
            {
                self.knot_error = self.move_knot(i, at.x, range).err();
            }
            if response.response.drag_stopped() {
                self.drag = None;
            }
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(f) = self.fits.get(self.selected) else {
            return Vec::new();
        };
        let Some(c) = self.current.iter().find(|c| c.id == f.id) else {
            return vec![("Result", "Awaiting a successful run".into())];
        };
        vec![
            ("Method", c.method.label().into()),
            ("Settings", c.settings.clone()),
            ("Residual norm", short(c.metrics[0])),
            ("Roughness", short(c.metrics[1])),
            ("Knot entries", c.knots.len().to_string()),
            ("Components", c.components.to_string()),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        for (i, f) in self.fits.iter().enumerate() {
            if ui
                .selectable_label(
                    self.selected == i,
                    format!("{} · {}", f.spline_name, f.method.label()),
                )
                .clicked()
            {
                self.selected = i;
                self.drag = None;
            }
            if let Some(c) = self.current.iter().find(|c| c.id == f.id) {
                crate::tokens::helper(
                    ui,
                    format!(
                        "{} · residual {} · roughness {} · {} knots",
                        c.settings,
                        short(c.metrics[0]),
                        short(c.metrics[1]),
                        c.knots.len()
                    ),
                );
            }
        }
        ui.add_space(crate::tokens::S);
        let Some(f) = self.fits.get_mut(self.selected) else {
            return;
        };
        text_field(ui, "Grid start (empty = first site)", &mut f.grid_start, 64);
        text_field(ui, "Grid end (empty = last site)", &mut f.grid_end, 64);
        field(ui, "Grid points", |ui| {
            ui.add(egui::DragValue::new(&mut f.grid_count).range(2..=MAX_GRID));
        });
        name_field(ui, "Spline structure", &mut f.spline_name, ws);
        name_field(ui, "Values on grid", &mut f.values_name, ws);
        crate::tokens::helper(
            ui,
            "Export and Generate script include all kept splines. Values have one row per component.",
        );
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        self.fits
            .iter()
            .flat_map(|f| [f.spline_name.clone(), f.values_name.clone()])
            .collect()
    }
    fn summary(&self) -> Option<String> {
        (!self.current.is_empty()).then(|| {
            format!(
                "Spline Tool: {} splines · last successful result",
                self.current.len()
            )
        })
    }
}
fn draw_curve(
    ui: &mut egui::Ui,
    c: &Curve,
    view: View,
    height: f32,
    ticks: &[f64],
    part: &'static str,
) -> egui_plot::PlotResponse<()> {
    let p = crate::tokens::palette(ui.ctx());
    egui_plot::Plot::new(("spline_tool_plot", part))
        .id(egui::Id::new(("spline_tool_plot", part)))
        .height(height)
        .show_background(false)
        .sense(egui::Sense::click_and_drag())
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .allow_axis_zoom_drag(false)
        .allow_double_click_reset(false)
        .legend(egui_plot::Legend::default())
        .x_axis_label("x")
        .y_axis_label(view.label())
        .show(ui, |plot| {
            let (xx, yy) = match view {
                View::Spline => (c.grid.as_slice(), c.values.as_slice()),
                View::First => (c.grid.as_slice(), c.first.as_slice()),
                View::Second => (c.grid.as_slice(), c.second.as_slice()),
                View::Residuals => (c.x.as_slice(), c.residuals.as_slice()),
            };
            for row in 0..c.components {
                let points = xx
                    .iter()
                    .enumerate()
                    .map(|(i, x)| [*x, yy[i * c.components + row]])
                    .collect::<Vec<_>>();
                let line = egui_plot::Line::new(format!("Row {}", row + 1), points)
                    .color(p.accent)
                    .width(2.);
                plot.line(if row.is_multiple_of(2) {
                    line
                } else {
                    line.style(egui_plot::LineStyle::dashed_dense())
                });
                if view == View::Spline {
                    let data =
                        c.x.iter()
                            .enumerate()
                            .map(|(i, x)| [*x, c.y[i * c.components + row]])
                            .collect::<Vec<_>>();
                    plot.points(
                        egui_plot::Points::new(format!("Data {}", row + 1), data)
                            .color(p.text_muted)
                            .radius(3.),
                    );
                }
            }
            // Data-derived tick baseline prevents automatic bounds from growing
            // each frame in response to ticks placed at the previous bounds.
            let (lo, hi) = yy
                .iter()
                .chain(if view == View::Spline {
                    c.y.iter()
                } else {
                    [].iter()
                })
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(*v), hi.max(*v))
                });
            let tick = (hi - lo).max(1e-6) * 0.04;
            for knot in ticks {
                plot.line(
                    egui_plot::Line::new("", vec![[*knot, lo], [*knot, lo + tick]])
                        .color(p.text_muted)
                        .width(2.),
                );
            }
        })
}
fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, limit: usize) -> bool {
    let mut changed = false;
    field(ui, label, |ui| {
        changed = ui
            .add(
                crate::tokens::input(egui::TextEdit::singleline(value).char_limit(limit))
                    .desired_width(ui.available_width()),
            )
            .changed();
    });
    changed
}
fn variable_ui(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    ws: &Snapshot,
    optional: bool,
    vector: bool,
) {
    field(ui, label, |ui| {
        egui::ComboBox::from_id_salt(label).selected_text(if value.is_empty() { "None" } else { value.as_str() }).width(ui.available_width()).show_ui(ui,|ui| {
            if optional { ui.selectable_value(value,String::new(),"None"); }
            for (name,array) in ws.variables() {
                if matches!(array,Array::Numeric(m) if !m.is_empty() && m.len() <= MAX_SITES*MAX_COMPONENTS && (!vector || m.rows()==1 || m.cols()==1)) { ui.selectable_value(value,name.to_owned(),name); }
            }
        });
    });
}
