//! Model order reduction. One script reduces a workspace tf/ss/zpk model and
//! plots the comparison; the private run appends read-only probes for the
//! views. A second private run reads the model's sizes and Hankel singular
//! values, so the bar chart and the order checks do not wait for a reduction.
use super::{
    App, AppKind, Outputs, Request, Runner, Snapshot, field, name_field, number, short, unused_name,
};
use eframe::egui;
use matrust::Array;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

/// Reduced models compared at once.
pub const MAX_ROWS: usize = 4;
/// The engine's model reduction limit.
pub const MAX_STATES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// balred: balanced truncation of the stable part.
    Balanced,
    /// modred: eliminate chosen states of ss(model).
    Modes,
    /// minreal: cancel pole-zero pairs.
    PoleZero,
}
impl Method {
    pub const ALL: [Self; 3] = [Self::Balanced, Self::Modes, Self::PoleZero];
    pub fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced truncation",
            Self::Modes => "Mode selection",
            Self::PoleZero => "Pole-zero simplification",
        }
    }
    fn function(self) -> &'static str {
        match self {
            Self::Balanced => "balred",
            Self::Modes => "modred",
            Self::PoleZero => "minreal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Step,
    Bode,
    PoleZero,
}
impl View {
    pub const ALL: [Self; 3] = [Self::Step, Self::Bode, Self::PoleZero];
    fn label(self) -> &'static str {
        match self {
            Self::Step => "Step",
            Self::Bode => "Bode",
            Self::PoleZero => "Pole-zero map",
        }
    }
}

/// One reduced model: how it is computed and the name it exports under.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Reduction {
    pub id: u32,
    pub name: String,
    pub export: bool,
    pub color: usize,
    pub method: Method,
    /// Balanced truncation and mode selection: keep the DC gain (singular
    /// perturbation) instead of truncating.
    pub match_dc: bool,
    /// Balanced truncation: the target order.
    pub order: String,
    /// The user set the order; otherwise it follows the suggested order.
    pub order_edited: bool,
    /// Mode selection: 1-based states of ss(model) to eliminate.
    pub eliminated: BTreeSet<usize>,
    /// Pole-zero simplification: empty uses minreal's default tolerance.
    pub tolerance: String,
}
impl Reduction {
    fn new(id: u32, name: String, method: Method) -> Self {
        Self {
            id,
            name,
            export: true,
            color: id as usize - 1,
            method,
            match_dc: true,
            order: "2".into(),
            order_edited: false,
            eliminated: BTreeSet::new(),
            tolerance: String::new(),
        }
    }
    fn elimination(&self) -> &'static str {
        if self.match_dc {
            "'MatchDC'"
        } else {
            "'Truncate'"
        }
    }
    /// The reduction call; `info` bounds orders and states. `opts` is the
    /// balredOptions variable used by balanced truncation.
    fn call(&self, model: &str, info: &ModelInfo, opts: &str) -> Result<String, String> {
        match self.method {
            Method::Balanced => {
                let r = integer(&self.order, "Target order")?;
                let (n, nu) = (info.states, info.unstable());
                if r > n {
                    return Err(format!("Target order must be at most {n}, the model order"));
                }
                if r < nu {
                    return Err(format!(
                        "Target order must be at least {nu}: unstable modes are always kept"
                    ));
                }
                Ok(format!("balred({model}, {r}, {opts})"))
            }
            Method::Modes => {
                if let Some(k) = self
                    .eliminated
                    .iter()
                    .find(|k| **k == 0 || **k > info.states)
                {
                    return Err(format!(
                        "State {k} does not exist; the model has {} states",
                        info.states
                    ));
                }
                let states: Vec<String> = self.eliminated.iter().map(usize::to_string).collect();
                Ok(format!(
                    "modred(ss({model}), [{}], {})",
                    states.join(" "),
                    self.elimination()
                ))
            }
            Method::PoleZero => {
                if self.tolerance.trim().is_empty() {
                    return Ok(format!("minreal({model})"));
                }
                let tol = literal(&self.tolerance, "Tolerance")?;
                if tol <= 0. {
                    return Err("Tolerance must be positive".into());
                }
                Ok(format!("minreal({model}, {})", number(tol)))
            }
        }
    }
    /// The order shown before the run: the target, the kept states or "?".
    fn planned_order(&self, info: Option<&ModelInfo>) -> String {
        match self.method {
            Method::Balanced => self.order.trim().to_owned(),
            Method::Modes => info.map_or("?".into(), |i| {
                i.states.saturating_sub(self.eliminated.len()).to_string()
            }),
            Method::PoleZero => "≤ n".into(),
        }
    }
}

/// One finite numeric literal, validated first as one expression.
fn literal(text: &str, what: &str) -> Result<f64, String> {
    if text.len() > 64 {
        return Err(format!("{what}: at most 64 characters"));
    }
    super::single_expression(text).map_err(|e| format!("{what}: {e}"))?;
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what}: enter one finite numeric literal"))
}
fn integer(text: &str, what: &str) -> Result<usize, String> {
    let v = literal(text, what)?;
    if v < 0. || v.fract() != 0. || v > MAX_STATES as f64 {
        return Err(format!("{what}: enter an integer from 0 to {MAX_STATES}"));
    }
    Ok(v as usize)
}

/// Sizes and Hankel singular values of the chosen model (Inf: unstable mode).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelInfo {
    pub outputs: usize,
    pub inputs: usize,
    pub states: usize,
    pub ts: f64,
    pub hsv: Vec<f64>,
}
impl ModelInfo {
    pub fn unstable(&self) -> usize {
        self.hsv.iter().filter(|v| v.is_infinite()).count()
    }
    /// Unstable modes plus half of the stable ones (at least 1 state).
    pub fn suggested_order(&self) -> usize {
        let nu = self.unstable();
        let stable = self.states.saturating_sub(nu);
        (nu + stable.div_ceil(2)).max(self.states.min(1))
    }
    fn mimo(&self) -> bool {
        self.outputs * self.inputs > 1
    }
}

#[derive(Default)]
struct InfoQuery {
    runner: Runner,
    /// The question the stored answer belongs to.
    key: Option<u64>,
    result: Option<Result<ModelInfo, String>>,
}

/// One model's curves on the shared step and frequency grids.
#[derive(Clone, Debug)]
pub struct Response {
    pub id: u32,
    pub name: String,
    /// Series colour; None is the original model.
    pub color: Option<usize>,
    pub order: usize,
    /// (t, y) for the chosen channel.
    pub step: Vec<[f64; 2]>,
    /// (log10 w, dB) and (log10 w, degrees).
    pub magnitude: Vec<[f64; 2]>,
    pub phase: Vec<[f64; 2]>,
    pub poles: Vec<[f64; 2]>,
    /// Zeros of the chosen channel.
    pub zeros: Vec<[f64; 2]>,
    /// 20 log10 max |G - Gr| over every channel on the Bode grid.
    pub error_db: f64,
    /// max |dcgain(G) - dcgain(Gr)| over every channel.
    pub dc_difference: f64,
}
#[derive(Clone, Debug)]
pub struct Comparison {
    pub model: String,
    pub channel: (usize, usize),
    pub original: Response,
    pub reduced: Vec<Response>,
}

pub struct ModelReducer {
    pub model: String,
    pub rows: Vec<Reduction>,
    pub selected: usize,
    /// Channel of the views (0-based output, input).
    pub output: usize,
    pub input: usize,
    pub view: View,
    pub current: Option<Comparison>,
    /// The Host is computing (set each frame by the Apps window).
    pub running: bool,
    /// Settings at Stop; nothing runs until they change or Run.
    stopped: Option<u64>,
    info: InfoQuery,
    /// Model whose order defaults the rows follow.
    last_model: String,
    next_id: u32,
}
impl Default for ModelReducer {
    fn default() -> Self {
        Self {
            model: String::new(),
            rows: vec![Reduction::new(1, "rsys".into(), Method::Balanced)],
            selected: 0,
            output: 0,
            input: 0,
            view: View::Step,
            current: None,
            running: false,
            stopped: None,
            info: InfoQuery::default(),
            last_model: String::new(),
            next_id: 2,
        }
    }
}

struct Plan {
    script: String,
    /// Row id and the script variable holding its reduced model.
    reduced: Vec<(u32, String)>,
}

const INFO_SIZE: &str = "mr_info_size";
const INFO_HSV: &str = "mr_info_hsv";

impl ModelReducer {
    pub fn add(&mut self, method: Method, ws: &Snapshot) -> Result<(), String> {
        if self.rows.len() >= MAX_ROWS {
            return Err(format!("At most {MAX_ROWS} reduced models"));
        }
        let name = unused_name("rsys", |n| {
            ws.contains(n) || n == self.model || self.rows.iter().any(|r| r.name == n)
        });
        let mut row = Reduction::new(self.next_id, name, method);
        let used: BTreeSet<usize> = self.rows.iter().map(|r| r.color).collect();
        row.color = (0..).find(|c| !used.contains(c)).unwrap_or(0);
        if let Some(info) = self.info() {
            row.order = info.suggested_order().to_string();
        }
        self.rows.push(row);
        self.next_id += 1;
        self.selected = self.rows.len() - 1;
        Ok(())
    }
    pub fn duplicate(&mut self, ws: &Snapshot) -> Result<(), String> {
        let Some(original) = self.rows.get(self.selected).cloned() else {
            return Ok(());
        };
        self.add(original.method, ws)?;
        let new = &mut self.rows[self.selected];
        let (id, name, color) = (new.id, new.name.clone(), new.color);
        *new = Reduction {
            id,
            name,
            color,
            ..original
        };
        Ok(())
    }
    pub fn remove(&mut self) {
        if self.selected < self.rows.len() {
            self.rows.remove(self.selected);
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
    /// Stop: cancels the run; nothing runs until an input changes or Run.
    pub fn halt(&mut self) {
        self.stopped = Some(self.fingerprint());
    }
    pub fn resume(&mut self) {
        self.stopped = None;
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }
    /// The app closed: the model query stops too.
    pub fn stop(&mut self) {
        self.info.runner.stop();
        self.info.key = None;
        self.info.result = None;
    }
    fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&self.model, &self.rows, self.output, self.input).hash(&mut hasher);
        hasher.finish()
    }

    // -- the model query ----------------------------------------------------

    /// Sizes and Hankel singular values of the chosen model, keyed by its
    /// workspace version.
    pub fn info_request(&self, ws: &Snapshot) -> Option<Request> {
        let array = ws.get(&self.model)?;
        if !ws.lti_models().any(|(name, _)| name == self.model) {
            return None;
        }
        let m = &self.model;
        let code = format!(
            "mr_info_s = ss({m});\n\
             {INFO_SIZE} = [size(mr_info_s.D, 1), size(mr_info_s.D, 2), size(mr_info_s.A, 1), mr_info_s.Ts];\n\
             if {INFO_SIZE}(3) > {MAX_STATES}, error('Model Reducer: models are limited to {MAX_STATES} states'); end\n\
             {INFO_HSV} = hsvd({m});\n"
        );
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        m.hash(&mut hasher);
        ws.version(m).hash(&mut hasher);
        Some(Request {
            key: hasher.finish(),
            inputs: vec![(m.clone(), array.clone())],
            code,
            outputs: vec![INFO_SIZE.into(), INFO_HSV.into()],
        })
    }
    /// Stores a finished model query; rows that follow the suggested order
    /// take it, and states the model lacks are no longer eliminated.
    pub fn receive_info(&mut self, key: u64, result: &Result<Outputs, String>) {
        let parsed = result.clone().and_then(|outputs| {
            let size = numbers(&outputs, INFO_SIZE)?;
            let hsv = numbers(&outputs, INFO_HSV)?;
            match size[..] {
                [p, m, n, ts] => Ok(ModelInfo {
                    outputs: p as usize,
                    inputs: m as usize,
                    states: n as usize,
                    ts,
                    hsv,
                }),
                _ => Err("Model Reducer: the model query gave no sizes".into()),
            }
        });
        if let Ok(info) = &parsed {
            for row in &mut self.rows {
                if !row.order_edited {
                    row.order = info.suggested_order().to_string();
                }
                row.eliminated.retain(|k| *k <= info.states);
            }
            self.output = self.output.min(info.outputs.saturating_sub(1));
            self.input = self.input.min(info.inputs.saturating_sub(1));
        }
        self.info.key = Some(key);
        self.info.result = Some(parsed);
    }
    /// Keeps the model query in step with the chosen model.
    pub fn poll_info(&mut self, ctx: &egui::Context, ws: &Snapshot) {
        let Some(request) = self.info_request(ws) else {
            self.info.runner.stop();
            return;
        };
        let key = request.key;
        if self.info.key == Some(key) {
            return;
        }
        self.info.runner.submit(ctx, request);
        if let Some(result) = self.info.runner.result() {
            let result = result.clone();
            self.receive_info(key, &result);
        }
    }
    /// The chosen model's sizes, once the query for its current value ended.
    pub fn info(&self) -> Option<&ModelInfo> {
        match (&self.info.result, self.info.key) {
            (Some(Ok(info)), Some(_)) => Some(info),
            _ => None,
        }
    }
    fn current_info(&self, ws: &Snapshot) -> Result<&ModelInfo, String> {
        let key = self.info_request(ws).map(|r| r.key);
        match (&self.info.result, key) {
            (Some(result), Some(key)) if self.info.key == Some(key) => match result {
                Ok(info) => Ok(info),
                Err(e) => Err(format!("Model Reducer: {}: {e}", self.model)),
            },
            _ => Err(format!("Model Reducer: reading {}…", self.model)),
        }
    }

    // -- the script ---------------------------------------------------------

    fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let model = &self.model;
        if model.is_empty() {
            return Err("Model Reducer: choose a tf, ss or zpk model from the workspace".into());
        }
        super::single_expression(model)?;
        if !matrust::files::valid_name(model) || !ws.lti_models().any(|(n, _)| n == model) {
            return Err(format!(
                "Model Reducer: {model} is not a tf, ss or zpk model in the workspace"
            ));
        }
        let info = self.current_info(ws)?;
        if self.rows.is_empty() {
            return Err("Model Reducer: add a reduced model".into());
        }
        if self.rows.len() > MAX_ROWS {
            return Err(format!("Model Reducer: at most {MAX_ROWS} reduced models"));
        }
        let mut names = BTreeSet::new();
        for row in &self.rows {
            super::single_expression(&row.name)?;
            if row.name.len() > 63 || !matrust::files::valid_name(&row.name) {
                return Err(
                    "Model Reducer: use valid variable names (at most 63 characters)".into(),
                );
            }
            super::check_export_name(&row.name)?;
            // The private run's probe calls functions after the exports.
            if matrust::session::FUNCTIONS.contains(&row.name.as_str()) {
                return Err(format!(
                    "Model Reducer: {} is a function name; choose another name",
                    row.name
                ));
            }
            if !names.insert(row.name.clone()) {
                return Err(format!("Model Reducer: duplicate export name {}", row.name));
            }
        }
        // Temporaries avoid workspace and output names; exports come last,
        // after every function call, so names such as `step` are safe.
        let mut taken = names.clone();
        taken.insert(model.clone());
        let mut fresh = |base: &str| {
            let name = unused_name(base, |n| ws.contains(n) || taken.contains(n));
            taken.insert(name.clone());
            name
        };
        let hsv = fresh("mr_hsv");
        let bars = fresh("mr_bars");
        let mut script = format!(
            "% Model Reducer: reduce {model} and compare the reduced models with it.\n\
             % Hankel singular values; Inf marks unstable modes, which balred always keeps.\n\
             {hsv} = hsvd({model});\n"
        );
        let mut opts_name: Option<String> = None;
        let mut reduced = Vec::new();
        for row in &self.rows {
            if row.method == Method::Balanced && opts_name.is_none() {
                opts_name = Some(fresh("opts"));
            }
            let opts = opts_name.as_deref().unwrap_or("opts");
            let call = row
                .call(model, info, opts)
                .map_err(|e| format!("Model Reducer: {}: {e}", row.name))?;
            let var = fresh("mr_reduced");
            script.push_str(&format!("% {}: {}\n", row.name, row.method.label()));
            if row.method == Method::Balanced {
                script.push_str(&format!(
                    "{opts} = balredOptions('StateProjection', {});\n",
                    row.elimination()
                ));
            }
            script.push_str(&format!("{var} = {call};\n"));
            reduced.push((row.id, var));
        }
        let all: Vec<&str> = std::iter::once(model.as_str())
            .chain(reduced.iter().map(|(_, v)| v.as_str()))
            .collect();
        let labels: Vec<String> = std::iter::once(model.as_str())
            .chain(self.rows.iter().map(|r| r.name.as_str()))
            .map(|n| format!("'{n}'"))
            .collect();
        let (all, labels) = (all.join(", "), labels.join(", "));
        script.push_str(&format!(
            "% Comparison plots.\n\
             figure;\n\
             {bars} = {hsv}; {bars}(~isfinite({bars})) = NaN;\n\
             bar({bars}); set(gca, 'YScale', 'log');\n\
             xlabel('State'); ylabel('Hankel singular value'); title('Hankel singular values of {model}');\n\
             figure; step({all}); legend({labels});\n\
             figure; bode({all}); legend({labels});\n"
        ));
        if info.mimo() {
            // pzmap takes SISO models only: plot the poles of each model.
            script.push_str("figure; hold on;\n");
            for (m, label) in std::iter::once(model.as_str())
                .chain(reduced.iter().map(|(_, v)| v.as_str()))
                .zip(
                    std::iter::once(model.as_str())
                        .chain(self.rows.iter().map(|r| r.name.as_str())),
                )
            {
                script.push_str(&format!(
                    "plot(real(pole({m})), imag(pole({m})), 'x', 'DisplayName', '{label}');\n"
                ));
            }
            script.push_str(
                "hold off; xlabel('Real'); ylabel('Imaginary'); title('Poles'); legend('show');\n",
            );
        } else {
            script.push_str(&format!(
                "figure; pzmap({all}); title('Pole-zero map: {}');\n",
                labels.replace('\'', "")
            ));
        }
        for (row, (_, var)) in self.rows.iter().zip(&reduced) {
            if row.export {
                script.push_str(&format!("{} = {var};\n", row.name));
            }
        }
        Ok(Plan { script, reduced })
    }

    /// Read-only lines after the script: curves of the chosen channel on
    /// the original's step and Bode grids, errors and DC differences.
    fn probe_text(&self, plan: &Plan, info: &ModelInfo) -> (String, Vec<String>) {
        let (i, j) = (self.output + 1, self.input + 1);
        let model = &self.model;
        let mut code = format!(
            "[mr_probe_y_0, mr_probe_t] = step({model});\n\
             [mr_probe_mag_0, mr_probe_ph_0, mr_probe_w] = bode({model});\n\
             mr_probe_h = freqresp({model}, mr_probe_w);\n\
             mr_probe_dc = dcgain({model});\n"
        );
        let mut outputs = vec!["mr_probe_t".to_owned(), "mr_probe_w".to_owned()];
        let models = std::iter::once((0, model.as_str()))
            .chain(plan.reduced.iter().map(|(id, v)| (*id, v.as_str())));
        for (id, m) in models {
            if id > 0 {
                code.push_str(&format!(
                    "mr_probe_y_{id} = step({m}, mr_probe_t);\n\
                     [mr_probe_mag_{id}, mr_probe_ph_{id}] = bode({m}, mr_probe_w);\n\
                     mr_probe_e_{id} = 20*log10(max(abs(reshape(freqresp({m}, mr_probe_w) - mr_probe_h, [], 1))));\n\
                     mr_probe_d_{id} = dcgain({m}) - mr_probe_dc; mr_probe_d_{id} = max(abs(mr_probe_d_{id}(:)));\n"
                ));
                outputs.extend([format!("mr_probe_e_{id}"), format!("mr_probe_d_{id}")]);
            }
            if info.mimo() {
                code.push_str(&format!(
                    "mr_probe_y_{id} = mr_probe_y_{id}(:, {i}, {j});\n\
                     mr_probe_mag_{id} = reshape(mr_probe_mag_{id}({i}, {j}, :), [], 1);\n\
                     mr_probe_ph_{id} = reshape(mr_probe_ph_{id}({i}, {j}, :), [], 1);\n"
                ));
            }
            let zeros = if info.mimo() {
                "zero(ss(mr_probe_s.A, mr_probe_s.B(:, J), mr_probe_s.C(I, :), mr_probe_s.D(I, J), mr_probe_s.Ts))"
                    .replace('I', &i.to_string())
                    .replace('J', &j.to_string())
            } else {
                format!("zero({m})")
            };
            code.push_str(&format!(
                "mr_probe_s = ss({m}); mr_probe_n_{id} = size(mr_probe_s.A, 1);\n\
                 mr_probe_p_{id} = pole({m}); mr_probe_p_{id} = [real(mr_probe_p_{id}(:)), imag(mr_probe_p_{id}(:))];\n\
                 mr_probe_z_{id} = {zeros}; mr_probe_z_{id} = [real(mr_probe_z_{id}(:)), imag(mr_probe_z_{id}(:))];\n"
            ));
            for part in ["y", "mag", "ph", "n", "p", "z"] {
                outputs.push(format!("mr_probe_{part}_{id}"));
            }
        }
        (code, outputs)
    }
}

fn numbers(outputs: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match outputs.get(name) {
        Some(Array::Numeric(m)) if !outputs[name].is_complex() => Ok(m.data().to_vec()),
        _ => Err(format!("Model Reducer: missing real result {name}")),
    }
}
/// An n x 2 [real imag] matrix as points.
fn complex_points(v: &[f64]) -> Vec<[f64; 2]> {
    let n = v.len() / 2;
    (0..n).map(|k| [v[k], v[n + k]]).collect()
}

impl App for ModelReducer {
    fn kind(&self) -> AppKind {
        AppKind::ModelReducer
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        if self.stopped.is_some() {
            return Err(
                "Model Reducer: stopped. Press Run or change an input to compute again.".into(),
            );
        }
        Ok(self.plan(ws)?.script)
    }
    fn inputs(&self) -> Vec<String> {
        if self.model.is_empty() {
            Vec::new()
        } else {
            vec![self.model.clone()]
        }
    }
    fn probe(&self, ws: &Snapshot) -> (String, Vec<String>) {
        match (self.plan(ws), self.current_info(ws)) {
            (Ok(plan), Ok(info)) => self.probe_text(&plan, info),
            _ => (String::new(), Vec::new()),
        }
    }
    fn receive(&mut self, outputs: &Outputs) -> Result<(), String> {
        let read = |name: &str| numbers(outputs, name);
        let t = read("mr_probe_t")?;
        let w = read("mr_probe_w")?;
        let curve = |id: u32, name: String, color: Option<usize>| -> Result<Response, String> {
            let part = |p: &str| read(&format!("mr_probe_{p}_{id}"));
            let y = part("y")?;
            let mag = part("mag")?;
            let ph = part("ph")?;
            if y.len() != t.len() || mag.len() != w.len() || ph.len() != w.len() {
                return Err("Model Reducer: response sizes disagree".into());
            }
            let order = part("n")?.first().copied().unwrap_or(0.) as usize;
            let (error_db, dc_difference) = if id == 0 {
                (f64::NAN, f64::NAN)
            } else {
                (
                    part("e")?.first().copied().unwrap_or(f64::NAN),
                    part("d")?.first().copied().unwrap_or(f64::NAN),
                )
            };
            Ok(Response {
                id,
                name,
                color,
                order,
                step: t.iter().zip(&y).map(|(t, y)| [*t, *y]).collect(),
                magnitude: w
                    .iter()
                    .zip(&mag)
                    .map(|(w, m)| [w.log10(), 20. * m.log10()])
                    .collect(),
                phase: w.iter().zip(&ph).map(|(w, p)| [w.log10(), *p]).collect(),
                poles: complex_points(&part("p")?),
                zeros: complex_points(&part("z")?),
                error_db,
                dc_difference,
            })
        };
        let original = curve(0, self.model.clone(), None)?;
        let reduced = self
            .rows
            .iter()
            .map(|r| curve(r.id, r.name.clone(), Some(r.color)))
            .collect::<Result<Vec<_>, String>>()?;
        self.current = Some(Comparison {
            model: self.model.clone(),
            channel: (self.output, self.input),
            original,
            reduced,
        });
        Ok(())
    }
    fn prepare(&mut self, ws: &Snapshot) {
        if self.model.is_empty()
            && let Some((name, _)) = ws.lti_models().next()
        {
            self.model = name.to_owned();
        }
        if self.model != self.last_model {
            // A new model: orders follow its suggestion, no state is
            // eliminated, and the old comparison no longer applies.
            for row in &mut self.rows {
                row.order_edited = false;
                row.eliminated.clear();
            }
            self.output = 0;
            self.input = 0;
            self.current = None;
            self.info.key = None;
            self.info.result = None;
            self.last_model = self.model.clone();
        }
        if self.stopped.is_some_and(|f| f != self.fingerprint()) {
            self.stopped = None;
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.poll_info(ui.ctx(), ws);
        let models: Vec<(String, &'static str)> =
            ws.lti_models().map(|(n, c)| (n.to_owned(), c)).collect();
        field(ui, "Model", |ui| {
            let response = egui::ComboBox::from_id_salt("mr_model")
                .width(ui.available_width())
                .selected_text(if self.model.is_empty() {
                    "Choose model"
                } else {
                    &self.model
                })
                .show_ui(ui, |ui| {
                    for (name, class) in &models {
                        ui.selectable_value(
                            &mut self.model,
                            name.clone(),
                            format!("{name} ({class})"),
                        );
                    }
                });
            crate::focus_areas::register(
                ui,
                &response.response,
                crate::focus_areas::Area::App(AppKind::ModelReducer),
            );
        });
        if models.is_empty() {
            crate::tokens::helper(ui, "Create a tf, ss or zpk model in the workspace first.");
        }
        if let Some(info) = self.info().cloned() {
            crate::tokens::helper(
                ui,
                format!(
                    "{} states ({} unstable) · {}×{} · {}",
                    info.states,
                    info.unstable(),
                    info.outputs,
                    info.inputs,
                    if info.ts == 0. {
                        "continuous".to_owned()
                    } else {
                        format!("Ts = {}", short(info.ts))
                    }
                ),
            );
            if info.mimo() {
                ui.add_space(crate::tokens::S);
                ui.horizontal(|ui| {
                    ui.label("Output");
                    channel_combo(ui, "mr_output", &mut self.output, info.outputs);
                    ui.label("Input");
                    channel_combo(ui, "mr_input", &mut self.input, info.inputs);
                });
            }
        }
        ui.add_space(crate::tokens::M);
        self.rows_ui(ui, ws);
        ui.add_space(crate::tokens::S);
        ui.horizontal(|ui| {
            if self.running && !self.is_stopped() {
                if ui.button("Stop").clicked() {
                    self.halt();
                }
                crate::tokens::helper(ui, "Computing…");
            } else if self.is_stopped() && ui.button("Run").clicked() {
                self.resume();
            }
        });
    }
    fn main_caption(&self) -> &'static str {
        "HANKEL SINGULAR VALUES AND RESPONSES"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.info().cloned() else {
            crate::tokens::helper(
                ui,
                if self.model.is_empty() {
                    "Choose a model to see its Hankel singular values."
                } else {
                    "Reading the model…"
                },
            );
            return;
        };
        let width = ui.available_width();
        let height = ui.available_height().max(360.);
        let bars = (height * 0.3).clamp(120., 240.);
        self.hsv_chart(ui, &info, width, bars);
        ui.add_space(crate::tokens::S);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(crate::tokens::XS, crate::tokens::XS);
            for view in View::ALL {
                if crate::tokens::tab(ui, self.view == view, view.label()).clicked() {
                    self.view = view;
                }
            }
        });
        ui.add_space(crate::tokens::S);
        let rest = (ui.available_height() - crate::tokens::S).max(160.);
        let Some(c) = &self.current else {
            crate::tokens::helper(ui, "The responses appear once the reduction computed.");
            return;
        };
        match self.view {
            View::Step => response_chart(ui, "mr_step", c, width, rest, Curve::Step),
            View::Bode => {
                let half = ((rest - crate::tokens::S) / 2.).max(80.);
                response_chart(ui, "mr_magnitude", c, width, half, Curve::Magnitude);
                ui.add_space(crate::tokens::S);
                response_chart(ui, "mr_phase", c, width, half, Curve::Phase);
            }
            View::PoleZero => roots_chart(ui, c, width, rest, info.ts != 0.),
        }
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let Some(info) = self.info() {
            rows.push(("Model", self.model.clone()));
            rows.push((
                "States",
                format!("{} ({} unstable)", info.states, info.unstable()),
            ));
            if info.mimo()
                && let Some((i, j)) = self.current.as_ref().map(|c| c.channel)
            {
                rows.push((
                    "Channel shown",
                    format!("output {} ← input {}", i + 1, j + 1),
                ));
            }
        }
        rows
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        self.table_ui(ui);
        ui.add_space(crate::tokens::M);
        let p = crate::tokens::palette(ui.ctx());
        ui.label(
            egui::RichText::new("Export")
                .font(crate::panels::caption_semibold(ui))
                .color(p.text_muted),
        );
        ui.add_space(crate::tokens::XS);
        for row in &mut self.rows {
            ui.checkbox(
                &mut row.export,
                format!("{} ({})", row.name, row.method.function()),
            );
            if row.export {
                name_field(ui, "Variable name", &mut row.name, ws);
            }
        }
        crate::tokens::helper(
            ui,
            "Unchecked models stay in the app. The script reduces and plots all of them.",
        );
    }
    fn exported(&self, _ws: &Snapshot) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.export)
            .map(|r| r.name.clone())
            .collect()
    }
    fn summary(&self) -> Option<String> {
        let c = self.current.as_ref()?;
        let best = c
            .reduced
            .iter()
            .map(|r| format!("{} order {}", r.name, r.order))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "Model Reducer: {} (order {}) → {best}",
            c.model, c.original.order
        ))
    }
}

fn channel_combo(ui: &mut egui::Ui, id: &str, value: &mut usize, count: usize) {
    egui::ComboBox::from_id_salt(id)
        .width(56.)
        .selected_text((*value + 1).to_string())
        .show_ui(ui, |ui| {
            for k in 0..count.min(64) {
                ui.selectable_value(value, k, (k + 1).to_string());
            }
        });
}
fn colour(index: usize) -> egui::Color32 {
    let [r, g, b] = matrust::plot::COLORS[index % matrust::plot::COLORS.len()];
    egui::Color32::from_rgb(r, g, b)
}
fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let mut changed = false;
    field(ui, label, |ui| {
        changed = ui
            .add(
                crate::tokens::input(egui::TextEdit::singleline(value).char_limit(64))
                    .desired_width(ui.available_width()),
            )
            .changed();
    });
    changed
}

impl ModelReducer {
    fn rows_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.label("Reduced models");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.rows.len() < MAX_ROWS, egui::Button::new("Add"))
                .clicked()
            {
                let _ = self.add(Method::Balanced, ws);
            }
            if ui
                .add_enabled(
                    !self.rows.is_empty() && self.rows.len() < MAX_ROWS,
                    egui::Button::new("Duplicate"),
                )
                .clicked()
            {
                let _ = self.duplicate(ws);
            }
            if ui
                .add_enabled(!self.rows.is_empty(), egui::Button::new("Remove"))
                .clicked()
            {
                self.remove();
            }
        });
        ui.add_space(crate::tokens::XS);
        for (k, row) in self.rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let response = crate::tokens::color_swatch(
                    ui,
                    crate::tokens::M,
                    colour(row.color),
                    egui::Sense::click(),
                );
                if response.on_hover_text("Change series colour").clicked() {
                    row.color = (row.color + 1) % matrust::plot::COLORS.len();
                }
                if ui.selectable_label(self.selected == k, &row.name).clicked() {
                    self.selected = k;
                }
            });
        }
        let info = self.info().cloned();
        let model = self.model.clone();
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        ui.add_space(crate::tokens::S);
        let mut method = row.method;
        field(ui, "Method", |ui| {
            egui::ComboBox::from_id_salt("mr_method")
                .width(ui.available_width())
                .selected_text(method.label())
                .show_ui(ui, |ui| {
                    for m in Method::ALL {
                        ui.selectable_value(&mut method, m, m.label());
                    }
                });
        });
        row.method = method;
        if matches!(row.method, Method::Balanced | Method::Modes) {
            field(ui, "State elimination", |ui| {
                egui::ComboBox::from_id_salt("mr_elimination")
                    .width(ui.available_width())
                    .selected_text(if row.match_dc {
                        "Match DC gain"
                    } else {
                        "Truncate"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut row.match_dc, true, "Match DC gain");
                        ui.selectable_value(&mut row.match_dc, false, "Truncate");
                    });
            });
        }
        match row.method {
            Method::Balanced => {
                if text_field(ui, "Target order", &mut row.order) {
                    row.order_edited = true;
                }
                crate::tokens::helper(
                    ui,
                    "Or click a bar in the chart. Unstable modes are always kept.",
                );
            }
            Method::Modes => match &info {
                Some(info) => {
                    ui.label("States of ss(model) to keep");
                    ui.horizontal_wrapped(|ui| {
                        for k in 1..=info.states {
                            let mut keep = !row.eliminated.contains(&k);
                            if ui.checkbox(&mut keep, format!("x{k}")).changed() {
                                if keep {
                                    row.eliminated.remove(&k);
                                } else {
                                    row.eliminated.insert(k);
                                }
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Keep all").clicked() {
                            row.eliminated.clear();
                        }
                        if let Ok(r) = integer(&row.order, "order")
                            && ui
                                .button(format!("Keep first {}", r.min(info.states)))
                                .clicked()
                        {
                            row.eliminated = (r + 1..=info.states).collect();
                        }
                    });
                }
                None => {
                    crate::tokens::helper(ui, "The states appear once the model was read.");
                }
            },
            Method::PoleZero => {
                text_field(ui, "Tolerance (empty: default)", &mut row.tolerance);
            }
        }
        let check = match &info {
            Some(info) => row.call(&model, info, "opts").map(|_| ()),
            None => Ok(()),
        };
        super::expression_message(ui, check);
    }

    /// Hankel singular values on a log axis: kept bars in the accent, removed
    /// ones muted, unstable modes (Inf) drawn at the top and always kept. A
    /// click sets the selected balanced truncation's target order.
    fn hsv_chart(&mut self, ui: &mut egui::Ui, info: &ModelInfo, width: f32, height: f32) {
        let p = crate::tokens::palette(ui.ctx());
        if info.hsv.is_empty() {
            crate::tokens::helper(ui, "The model has no states.");
            return;
        }
        let finite: Vec<f64> = info
            .hsv
            .iter()
            .copied()
            .filter(|v| v.is_finite() && *v > 0.)
            .collect();
        let top = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let bottom = finite.iter().copied().fold(f64::INFINITY, f64::min);
        let (top, bottom) = if finite.is_empty() {
            (0., -1.)
        } else {
            (top.log10(), bottom.log10())
        };
        let floor = (bottom - 1.).floor();
        let ceiling = top + 0.5;
        let row = self.rows.get(self.selected);
        let kept = row
            .filter(|r| r.method == Method::Balanced)
            .and_then(|r| integer(&r.order, "order").ok());
        let (mut keep, mut drop, mut unstable) = (Vec::new(), Vec::new(), Vec::new());
        for (k, v) in info.hsv.iter().enumerate() {
            let x = (k + 1) as f64;
            if v.is_infinite() {
                unstable.push(
                    egui_plot::Bar::new(x, ceiling - floor)
                        .base_offset(floor)
                        .width(0.7),
                );
                continue;
            }
            let h = if *v > 0. { v.log10() } else { floor };
            let bar = egui_plot::Bar::new(x, h - floor)
                .base_offset(floor)
                .width(0.7);
            match kept {
                Some(r) if k < r => keep.push(bar),
                Some(_) => drop.push(bar),
                None => keep.push(bar),
            }
        }
        let response = egui_plot::Plot::new("mr_hsv")
            .width(width)
            .height(height)
            .show_background(false)
            .allow_scroll(false)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .legend(egui_plot::Legend::default())
            .include_y(floor)
            .include_y(ceiling)
            .x_axis_label("State")
            .y_axis_label("Hankel singular value")
            .y_axis_formatter(|mark, _| short(10_f64.powf(mark.value)))
            .show(ui, |plot| {
                let kept_label = if kept.is_some() { "Kept" } else { "Stable" };
                plot.bar_chart(egui_plot::BarChart::new(kept_label, keep).color(p.accent));
                if !drop.is_empty() {
                    plot.bar_chart(
                        egui_plot::BarChart::new("Removed", drop).color(p.text_disabled),
                    );
                }
                if !unstable.is_empty() {
                    plot.bar_chart(
                        egui_plot::BarChart::new("Unstable (always kept)", unstable)
                            .color(p.warning),
                    );
                }
                plot.pointer_coordinate()
            });
        if response.response.clicked()
            && let Some(point) = response.inner
        {
            self.click_bar(point.x, info);
        }
    }
    /// A click at `x` on the bar chart keeps bars 1..=x (and every unstable one).
    pub fn click_bar(&mut self, x: f64, info: &ModelInfo) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        if row.method != Method::Balanced || info.states == 0 {
            return;
        }
        let k = (x.round().max(1.) as usize).min(info.states);
        row.order = k.max(info.unstable()).to_string();
        row.order_edited = true;
    }

    /// Method, order, error and DC difference per reduced model.
    fn table_ui(&mut self, ui: &mut egui::Ui) {
        let p = crate::tokens::palette(ui.ctx());
        let info = self.info().cloned();
        let shown = self.current.as_ref();
        let mut chosen = None;
        let error_heading = if ui.available_width() < super::COLUMN {
            "Max\n|error| dB"
        } else {
            "Max |error| dB"
        };
        egui::Grid::new("mr_table")
            .num_columns(5)
            .min_col_width(0.)
            .max_col_width(ui.available_width())
            .spacing(egui::Vec2::splat(crate::tokens::XS))
            .show(ui, |ui| {
                for heading in ["Model", "Method", "Order", error_heading, "DC diff"] {
                    ui.label(
                        egui::RichText::new(heading)
                            .size(crate::tokens::CAPTION)
                            .color(p.text_muted),
                    );
                }
                ui.end_row();
                for (k, row) in self.rows.iter().enumerate() {
                    if ui.selectable_label(k == self.selected, &row.name).clicked() {
                        chosen = Some(k);
                    }
                    ui.label(row.method.function());
                    match shown.and_then(|c| c.reduced.iter().find(|r| r.id == row.id)) {
                        Some(r) => {
                            ui.label(r.order.to_string());
                            ui.label(short(r.error_db));
                            ui.label(short(r.dc_difference));
                        }
                        None => {
                            ui.label(row.planned_order(info.as_ref()));
                            ui.label("—");
                            ui.label("—");
                        }
                    }
                    ui.end_row();
                }
            });
        if let Some(k) = chosen {
            self.selected = k;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Curve {
    Step,
    Magnitude,
    Phase,
}

fn response_chart(
    ui: &mut egui::Ui,
    id: &str,
    c: &Comparison,
    width: f32,
    height: f32,
    curve: Curve,
) {
    let p = crate::tokens::palette(ui.ctx());
    let log_x = curve != Curve::Step;
    let (x, y) = match curve {
        Curve::Step => ("Time", "Amplitude"),
        Curve::Magnitude => ("Frequency", "Magnitude (dB)"),
        Curve::Phase => ("Frequency", "Phase (deg)"),
    };
    egui_plot::Plot::new(id)
        .width(width)
        .height(height)
        .show_background(false)
        .allow_scroll(false)
        .legend(egui_plot::Legend::default())
        .x_axis_label(x)
        .y_axis_label(y)
        .x_axis_formatter(move |mark, _| {
            short(if log_x {
                10_f64.powf(mark.value)
            } else {
                mark.value
            })
        })
        .show(ui, |plot| {
            for r in std::iter::once(&c.original).chain(&c.reduced) {
                let points = match curve {
                    Curve::Step => &r.step,
                    Curve::Magnitude => &r.magnitude,
                    Curve::Phase => &r.phase,
                };
                let points: Vec<[f64; 2]> = points
                    .iter()
                    .copied()
                    .filter(|[a, b]| a.is_finite() && b.is_finite())
                    .collect();
                let line = egui_plot::Line::new(&r.name, points);
                plot.line(match r.color {
                    Some(k) => line.color(colour(k)).width(1.5),
                    None => line
                        .color(p.text)
                        .width(1.5)
                        .style(egui_plot::LineStyle::dashed_loose()),
                });
            }
        });
}

fn roots_chart(ui: &mut egui::Ui, c: &Comparison, width: f32, height: f32, discrete: bool) {
    let p = crate::tokens::palette(ui.ctx());
    egui_plot::Plot::new("mr_roots")
        .width(width)
        .height(height)
        .show_background(false)
        .allow_scroll(false)
        .data_aspect(1.)
        .legend(egui_plot::Legend::default())
        .x_axis_label("Real")
        .y_axis_label("Imaginary")
        .show(ui, |plot| {
            if discrete {
                let circle: Vec<[f64; 2]> = (0..=180)
                    .map(|k| {
                        let a = std::f64::consts::TAU * k as f64 / 180.;
                        [a.cos(), a.sin()]
                    })
                    .collect();
                plot.line(
                    egui_plot::Line::new("Unit circle", circle)
                        .color(p.border)
                        .width(1.),
                );
            }
            for r in std::iter::once(&c.original).chain(&c.reduced) {
                let color = r.color.map_or(p.text, colour);
                let finite = |v: &[[f64; 2]]| -> Vec<[f64; 2]> {
                    v.iter()
                        .copied()
                        .filter(|[a, b]| a.is_finite() && b.is_finite())
                        .collect()
                };
                plot.points(
                    egui_plot::Points::new(format!("{} poles", r.name), finite(&r.poles))
                        .shape(egui_plot::MarkerShape::Cross)
                        .radius(5.)
                        .color(color),
                );
                plot.points(
                    egui_plot::Points::new(format!("{} zeros", r.name), finite(&r.zeros))
                        .shape(egui_plot::MarkerShape::Circle)
                        .filled(false)
                        .radius(5.)
                        .color(color),
                );
            }
        });
}
