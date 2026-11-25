//! Clean-room BER workbench. All numerical results come from canonical engine scripts.
use super::{
    App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, number, short,
};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::{Array, Engine};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub fn checked_name(s: &str) -> Result<(), String> {
    if !matrust::files::valid_name(s) || s.starts_with("app_comm_") {
        return Err("Use a valid variable name outside app_comm_".into());
    }
    super::single_expression(s)?;
    matrust::syntax::parse(&format!("{s}=0;"))?;
    super::check_export_name(s)
}
/// Parse with the real parser, permit only bounded arithmetic/vector syntax, then
/// let the engine evaluate it under a small allocation and evaluation budget.
pub fn numeric_text(s: &str, max: usize) -> Result<Vec<f64>, String> {
    use matrust::syntax::{Expr, StatementKind, Token};
    fn safe(e: &Expr, depth: usize) -> bool {
        if depth > 24 {
            return false;
        }
        match e {
            Expr::Number(v) => v.is_finite(),
            Expr::Unary(_, x) | Expr::Transpose(x) | Expr::NonConjugateTranspose(x) => {
                safe(x, depth + 1)
            }
            Expr::Binary(
                Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::DotStar
                | Token::DotSlash
                | Token::Power
                | Token::DotPower,
                x,
                y,
            ) => safe(x, depth + 1) && safe(y, depth + 1),
            Expr::Range(a, b, c) => {
                safe(a, depth + 1)
                    && b.as_ref().is_none_or(|b| safe(b, depth + 1))
                    && safe(c, depth + 1)
            }
            Expr::Matrix(rows) => rows.iter().flatten().all(|e| safe(e, depth + 1)),
            Expr::Call(name, args)
                if matches!(name.as_str(), "linspace" | "logspace") && args.len() <= 3 =>
            {
                args.iter().all(|e| safe(e, depth + 1))
            }
            _ => false,
        }
    }
    if s.len() > 2048 {
        return Err("Numeric expression exceeds 2048 bytes".into());
    }
    super::single_expression(s)?;
    let code = format!("v=({s});");
    let ast = matrust::syntax::parse(&code)?;
    if !matches!(&ast[0].kind, StatementKind::Evaluate(e) if safe(&e.expr, 0)) {
        return Err(
            "Use numeric literals, arithmetic, vectors, colon ranges, linspace or logspace".into(),
        );
    }
    let limits = matrust::engine::Limits {
        memory_bytes: 256 * 1024,
        evaluation_units: Some(32768),
        ..Default::default()
    };
    let mut e = Engine::with_limits(limits);
    e.execute(&code)?;
    let Some(Array::Numeric(v)) = e.array("v") else {
        return Err("Expected a real numeric vector".into());
    };
    if (v.rows() > 1 && v.cols() > 1) || v.len() > max || v.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("Use a finite vector of at most {max} values"));
    }
    Ok(v.data().to_vec())
}
pub fn vector(v: &[f64]) -> String {
    format!(
        "[{}]",
        v.iter().map(|v| number(*v)).collect::<Vec<_>>().join(" ")
    )
}
pub fn numbers(o: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match o.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Missing numeric result {name}")),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modulation {
    Psk,
    Qam,
    Pam,
    Fsk,
    Dpsk,
    Msk,
    Oqpsk,
}
impl Modulation {
    pub const THEORY: [Self; 7] = [
        Self::Psk,
        Self::Dpsk,
        Self::Oqpsk,
        Self::Pam,
        Self::Qam,
        Self::Fsk,
        Self::Msk,
    ];
    pub const SIM: [Self; 6] = [
        Self::Psk,
        Self::Qam,
        Self::Pam,
        Self::Fsk,
        Self::Dpsk,
        Self::Msk,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Psk => "PSK",
            Self::Qam => "QAM",
            Self::Pam => "PAM",
            Self::Fsk => "FSK",
            Self::Dpsk => "DPSK",
            Self::Msk => "MSK",
            Self::Oqpsk => "OQPSK",
        }
    }
    pub fn order_ok(self, m: u32) -> bool {
        m.is_power_of_two()
            && (2..=1024).contains(&m)
            && (self != Self::Qam || m >= 4)
            && (self != Self::Msk || m == 2)
            && (self != Self::Oqpsk || m == 4)
    }
}
pub fn modulation_ui(
    ui: &mut egui::Ui,
    id: &str,
    kind: &mut Modulation,
    choices: &[Modulation],
    order: &mut u32,
    max_order: u32,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(kind.label())
        .show_ui(ui, |ui| {
            for &k in choices {
                if ui.selectable_value(kind, k, k.label()).changed() && !k.order_ok(*order) {
                    *order = if k == Modulation::Msk { 2 } else { 4 };
                }
            }
        });
    egui::ComboBox::from_id_salt((id, "order"))
        .selected_text(format!("Order {}", *order))
        .show_ui(ui, |ui| {
            for m in [2, 4, 8, 16, 32, 64, 128, 256, 512, 1024] {
                ui.add_enabled_ui(kind.order_ok(m) && m <= max_order, |ui| {
                    ui.selectable_value(order, m, m.to_string());
                });
            }
        });
}
#[derive(Clone, Debug)]
pub struct Spec {
    pub ebno: String,
    pub simulation: bool,
    pub modulation: Modulation,
    pub order: u32,
    pub coherent: bool,
    pub differential: bool,
    pub target_errors: u32,
    pub max_bits: u32,
    pub seed: u32,
    pub fit: bool,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            ebno: "0:2:10".into(),
            simulation: false,
            modulation: Modulation::Qam,
            order: 16,
            coherent: true,
            differential: false,
            target_errors: 500,
            max_bits: 200_000,
            seed: 7,
            fit: false,
        }
    }
}
impl Spec {
    pub fn grid(&self) -> Result<Vec<f64>, String> {
        let v = numeric_text(&self.ebno, 128)?;
        if v.is_empty()
            || v.iter().any(|x| !(-30.0..=60.0).contains(x))
            || v.windows(2).any(|w| w[0] >= w[1])
        {
            return Err("Eb/N0 must increase, with 1–128 values in −30…60 dB".into());
        }
        self.validate()?;
        if !self.simulation
            && v.len()
                .saturating_mul(self.order as usize)
                .saturating_mul(self.order as usize)
                > 16_777_216
        {
            return Err(
                "Theory grid exceeds the engine integration work limit; reduce points or order"
                    .into(),
            );
        }
        Ok(v)
    }
    fn validate(&self) -> Result<(), String> {
        if !self.modulation.order_ok(self.order)
            || (self.simulation && self.modulation == Modulation::Oqpsk)
            || (!self.simulation
                && self.modulation == Modulation::Fsk
                && !self.coherent
                && self.order > 64)
            || self.max_bits < 64
            || self.max_bits > 2_000_000
            || self.target_errors == 0
            || self.target_errors > 1_000_000
        {
            return Err("Choose a valid modulation, 1–1000000 errors and 64–2000000 bits".into());
        }
        Ok(())
    }
    fn theory(&self, x: &str) -> String {
        let diff = if self.differential { "diff" } else { "nondiff" };
        let coherence = if self.coherent {
            "coherent"
        } else {
            "noncoherent"
        };
        match self.modulation {
            Modulation::Psk => format!(
                "berawgn({x},'psk',{},'{}')",
                self.order,
                if self.order <= 4 { diff } else { "nondiff" }
            ),
            Modulation::Oqpsk => format!("berawgn({x},'oqpsk','{diff}')"),
            Modulation::Msk => format!(
                "berawgn({x},'msk','{}','{coherence}')",
                if self.differential { "off" } else { "on" }
            ),
            Modulation::Fsk => format!("berawgn({x},'fsk',{},'{coherence}')", self.order),
            k => format!(
                "berawgn({x},'{}',{})",
                k.label().to_ascii_lowercase(),
                self.order
            ),
        }
    }
}
/// The sole BER script builder: one function for each kept dataset, common
/// modem calls, deterministic point seeds, and exactly the recorded bit limits.
pub fn build_script(spec: &Spec, grid: &[f64], name: &str) -> Result<String, String> {
    build_sets(&[(spec.clone(), grid.to_vec(), false)], name)
}
fn build_sets(sets: &[(Spec, Vec<f64>, bool)], name: &str) -> Result<String, String> {
    checked_name(name)?;
    if sets.is_empty() || sets.len() > 32 {
        return Err("Select 1–32 datasets".into());
    }
    let mut code = format!(
        "{name}=[{}];\n",
        (0..sets.len())
            .map(|i| format!("app_comm_ber_{i}()"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for (i, (s, grid, partial)) in sets.iter().enumerate() {
        s.validate()?;
        if grid.is_empty()
            || grid.len() > 128
            || grid
                .iter()
                .any(|x| !x.is_finite() || !(-30.0..=60.0).contains(x))
        {
            return Err("Invalid Eb/N0 grid".into());
        }
        code += &format!(
            "function r=app_comm_ber_{i}()\nx={};\nb=zeros(size(x)); ser=[]; errors=zeros(size(x)); bits=zeros(size(x));\n",
            vector(grid)
        );
        if s.simulation {
            let k = s.order.ilog2();
            let ns = match s.modulation {
                Modulation::Fsk => 2 * s.order,
                Modulation::Msk => 8,
                _ => 1,
            };
            let (md, dm) = modem_calls(s.modulation, s.order, ns, "d", "z");
            let batch = if s.modulation == Modulation::Fsk {
                (10_000_000 / (ns * s.order)).clamp(1, 2048)
            } else {
                2048
            };
            let extra = if s.modulation == Modulation::Dpsk {
                1
            } else {
                0
            };
            let compare = if s.modulation == Modulation::Dpsk {
                "[ne,rate]=biterr(d(2:end),dh(2:end),k); nb=k*(n-1);"
            } else {
                "[ne,rate]=biterr(d,dh,k); nb=k*n;"
            };
            code += &format!(
                "k={k};\nfor j=1:numel(x)\nrng(mod({}+j-1,4294967296));\nwhile errors(j)<{} && bits(j)+k<={}\nn=min({batch},floor(({}-bits(j))/k))+{extra};\nsource_bits=randi([0 1],k*n,1); d=bit2int(source_bits,k);\ntx={md};\nz=awgn(tx,x(j)+10*log10(k)-10*log10({ns}),'measured');\ndh={dm};\n{compare}\nerrors(j)=errors(j)+ne; bits(j)=bits(j)+nb;\nend\nb(j)=errors(j)/bits(j);\nend\n",
                s.seed, s.target_errors, s.max_bits, s.max_bits
            );
        } else {
            code += &format!("[b,ser]={};\n", s.theory("x"));
        }
        code += "fitx=[]; fity=[];\n";
        if s.fit && s.simulation && grid.len() >= 4 {
            code += "valid=find(b>0); if numel(valid)>=4, fitx=x(valid); fity=berfit(fitx,b(valid),fitx,[],'exp'); end\n";
        }
        code += &format!(
            "settings=struct('Modulation','{}','Order',{},'MonteCarlo',{},'Coherent',{},'Differential',{},'TargetErrors',{},'MaxBits',{},'Seed',{},'Partial',{},'Fit',{});\nr=struct('EbNo',x,'BER',b,'SER',ser,'Errors',errors,'Bits',bits,'settings',settings,'FitEbNo',fitx,'FitBER',fity);\nend\n",
            s.modulation.label(),
            s.order,
            u8::from(s.simulation),
            u8::from(s.coherent),
            u8::from(s.differential),
            s.target_errors,
            s.max_bits,
            s.seed,
            u8::from(*partial),
            u8::from(s.fit)
        );
    }
    Ok(code)
}
pub fn modem_calls(
    kind: Modulation,
    m: u32,
    ns: u32,
    data: &str,
    signal: &str,
) -> (String, String) {
    match kind {
        Modulation::Psk | Modulation::Qam | Modulation::Pam => {
            let k = kind.label().to_ascii_lowercase();
            let opt = if matches!(kind, Modulation::Psk | Modulation::Pam) {
                ",0,'gray'"
            } else {
                ",'gray'"
            };
            (
                format!("{k}mod({data},{m}{opt})"),
                format!("{k}demod({signal},{m}{opt})"),
            )
        }
        Modulation::Dpsk => (
            format!("dpskmod({data},{m},0,'gray')"),
            format!("dpskdemod({signal},{m},0,'gray')"),
        ),
        Modulation::Fsk => (
            format!("fskmod({data},{m},1,{ns},{ns},'cont','gray')"),
            format!("fskdemod({signal},{m},1,{ns},{ns},'gray')"),
        ),
        Modulation::Msk => (
            format!("mskmod({data},{ns},'nondiff')"),
            format!("mskdemod({signal},{ns},'nondiff')"),
        ),
        Modulation::Oqpsk => unreachable!(),
    }
}
#[derive(Clone)]
pub struct Dataset {
    pub label: String,
    pub visible: bool,
    pub selected: bool,
    pub spec: Spec,
    pub x: Vec<f64>,
    pub ber: Vec<f64>,
    pub ser: Vec<f64>,
    pub errors: Vec<f64>,
    pub bits: Vec<f64>,
    pub partial: bool,
    pub fit: Vec<[f64; 2]>,
}
#[derive(Default)]
struct Progress {
    points: usize,
    latest: Option<Dataset>,
}
pub struct BerAnalysis {
    pub spec: Spec,
    pub datasets: Vec<Dataset>,
    pub selected: usize,
    pub name: String,
    pub error: Option<String>,
    pub ser_view: bool,
    pub stop: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    cancellation: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    pending: Arc<Mutex<Option<Dataset>>>,
    active: Option<(Spec, Vec<f64>)>,
    serial: u64,
    pub running: bool,
    launch_pending: AtomicBool,
    pub fit_requested: bool,
}
impl Default for BerAnalysis {
    fn default() -> Self {
        Self {
            spec: Spec::default(),
            datasets: vec![],
            selected: 0,
            name: "ber_results".into(),
            error: None,
            ser_view: false,
            stop: Arc::default(),
            progress: Arc::default(),
            cancellation: Arc::default(),
            pending: Arc::default(),
            active: None,
            serial: 0,
            running: false,
            launch_pending: AtomicBool::new(false),
            fit_requested: false,
        }
    }
}
impl BerAnalysis {
    #[cfg(test)]
    pub fn completed_points(&self) -> usize {
        self.progress.lock().unwrap().points
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(cancel) = self.cancellation.lock().unwrap().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn start(&mut self) -> Result<(), String> {
        self.spec.fit = self.fit_requested && self.spec.simulation;
        let grid = self.spec.grid()?;
        if self.datasets.len() >= 32 {
            return Err("Keep at most 32 datasets; delete one before adding".into());
        }
        self.active = Some((self.spec.clone(), grid));
        self.serial += 1;
        self.stop.store(false, Ordering::Relaxed);
        *self.progress.lock().unwrap() = Progress::default();
        *self.pending.lock().unwrap() = None;
        self.running = true;
        self.launch_pending.store(true, Ordering::Relaxed);
        self.error = None;
        Ok(())
    }
    fn selected_sets(&self) -> Vec<(Spec, Vec<f64>, bool)> {
        self.datasets
            .iter()
            .filter(|d| d.selected)
            .map(|d| (d.spec.clone(), d.x.clone(), d.partial))
            .collect()
    }
}
impl App for BerAnalysis {
    fn kind(&self) -> AppKind {
        AppKind::BitErrorRateAnalysis
    }
    fn script(&self, _: &Snapshot) -> Result<String, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        if self.datasets.iter().any(|d| d.selected) {
            build_sets(&self.selected_sets(), &self.name)
        } else {
            let (s, x) = self
                .active
                .clone()
                .unwrap_or((self.spec.clone(), self.spec.grid()?));
            build_script(&s, &x, &self.name)
        }
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        if self.selected_sets().is_empty() {
            return Err("Select a kept dataset".into());
        }
        self.script(ws)
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (String::new(), vec![])
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, _: Request) {
        self.launch_pending.store(false, Ordering::Relaxed);
        if !self.running || runner.key() == Some(self.serial) {
            runner.poll(ctx);
            return;
        }
        let Some((spec, grid)) = self.active.clone() else {
            return;
        };
        let pending = self.pending.clone();
        let progress = self.progress.clone();
        let cancellation_slot = self.cancellation.clone();
        let stop = self.stop.clone();
        let repaint = ctx.clone();
        let fit = self.fit_requested;
        runner.submit_task(
            ctx,
            self.serial,
            Box::new(move |e| {
                let cancellation = e.cancellation();
                *cancellation_slot
                    .lock()
                    .map_err(|_| "BER cancellation lock failed")? = Some(cancellation.clone());
                let mut d = Dataset {
                    label: format!(
                        "{} {} {}",
                        spec.modulation.label(),
                        spec.order,
                        if spec.simulation {
                            "simulation"
                        } else {
                            "theory"
                        }
                    ),
                    visible: true,
                    selected: true,
                    spec: spec.clone(),
                    x: vec![],
                    ber: vec![],
                    ser: vec![],
                    errors: vec![],
                    bits: vec![],
                    partial: false,
                    fit: vec![],
                };
                for (i, &x) in grid.iter().enumerate() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut point = spec.clone();
                    point.seed = spec.seed.wrapping_add(i as u32);
                    point.fit = false;
                    let code = build_script(&point, &[x], "r")?;
                    let (body, functions) = super::split_local_functions(&code);
                    let code =
                        format!("{body}v=[r.EbNo;r.BER;r.Errors;r.Bits]; s=r.SER;\n{functions}");
                    if let Err(error) = e.execute(&code) {
                        if stop.load(Ordering::Relaxed) {
                            cancellation.store(false, Ordering::Relaxed);
                            break;
                        }
                        return Err(error);
                    }
                    let Some(Array::Numeric(v)) = e.array("v") else {
                        return Err("Missing BER results".into());
                    };
                    d.x.push(x);
                    d.ber.push(v.data()[1]);
                    d.errors.push(v.data()[2]);
                    d.bits.push(v.data()[3]);
                    if let Some(Array::Numeric(s)) = e.array("s") {
                        d.ser.extend(s.data());
                    }
                    *progress.lock().map_err(|_| "BER progress lock failed")? = Progress {
                        points: d.x.len(),
                        latest: Some(d.clone()),
                    };
                    repaint.request_repaint();
                }
                *cancellation_slot
                    .lock()
                    .map_err(|_| "BER cancellation lock failed")? = None;
                cancellation.store(false, Ordering::Relaxed);
                d.partial = d.x.len() < grid.len();
                let valid: Vec<_> = d.x.iter().zip(&d.ber).filter(|(_, y)| **y > 0.).collect();
                if fit && valid.len() >= 4 {
                    e.execute(&format!(
                        "fx={}; fy=berfit(fx,{},fx,[],'exp');",
                        vector(&valid.iter().map(|p| *p.0).collect::<Vec<_>>()),
                        vector(&valid.iter().map(|p| *p.1).collect::<Vec<_>>())
                    ))?;
                    if let (Some(Array::Numeric(x)), Some(Array::Numeric(y))) =
                        (e.array("fx"), e.array("fy"))
                    {
                        d.fit = x
                            .data()
                            .iter()
                            .zip(y.data())
                            .map(|(&x, &y)| [x, y])
                            .collect();
                    }
                }
                *pending.lock().map_err(|_| "BER result lock failed")? = Some(d);
                Ok(Outputs::new())
            }),
        );
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        self.running = false;
        if let Some(d) = self
            .pending
            .lock()
            .map_err(|_| "BER result lock failed")?
            .take()
            && !d.x.is_empty()
        {
            for old in &mut self.datasets {
                old.selected = false;
            }
            self.datasets.push(d);
            self.selected = self.datasets.len() - 1;
        }
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, _: &Snapshot) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.spec.simulation, false, "Theoretical");
            ui.selectable_value(&mut self.spec.simulation, true, "Monte Carlo");
        });
        if self.spec.simulation && self.spec.modulation == Modulation::Oqpsk {
            self.spec.modulation = Modulation::Psk;
        }
        let order_limit = if self.spec.modulation == Modulation::Fsk
            && !self.spec.coherent
            && !self.spec.simulation
        {
            64
        } else {
            1024
        };
        modulation_ui(
            ui,
            "ber_mod",
            &mut self.spec.modulation,
            if self.spec.simulation {
                &Modulation::SIM
            } else {
                &Modulation::THEORY
            },
            &mut self.spec.order,
            order_limit,
        );
        if self.spec.modulation == Modulation::Fsk && self.spec.order > 64 {
            self.spec.coherent = true;
        }
        if self.spec.modulation == Modulation::Psk && self.spec.order > 4 {
            self.spec.differential = false;
        }
        field(ui, "Eb/N0 (dB)", |ui| {
            ui.add(crate::tokens::input(
                egui::TextEdit::singleline(&mut self.spec.ebno).char_limit(2048),
            ));
        });
        super::expression_message(ui, self.spec.grid().map(|_| ()));
        if self.spec.simulation {
            field(ui, "Target errors", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.target_errors).range(1..=1_000_000));
            });
            field(ui, "Maximum bits per point", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.max_bits).range(64..=2_000_000));
            });
            field(ui, "Seed", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.seed));
            });
            ui.checkbox(&mut self.fit_requested, "Fit with berfit (4+ points)");
        } else {
            ui.add_enabled_ui(
                self.spec.modulation == Modulation::Msk
                    || (self.spec.modulation == Modulation::Fsk && self.spec.order <= 64),
                |ui| {
                    ui.checkbox(&mut self.spec.coherent, "Coherent");
                },
            );
            ui.add_enabled_ui(
                (self.spec.modulation == Modulation::Psk && self.spec.order <= 4)
                    || matches!(self.spec.modulation, Modulation::Oqpsk | Modulation::Msk),
                |ui| {
                    ui.checkbox(&mut self.spec.differential, "Differential encoding");
                },
            );
        }
        if ui
            .add_enabled(!self.running, egui::Button::new("Add dataset"))
            .clicked()
        {
            self.error = self.start().err();
            if self.running {
                ui.ctx().request_repaint();
            }
        }
        if ui
            .add_enabled(self.running, egui::Button::new("Stop"))
            .clicked()
        {
            self.request_stop();
        }
        if self.running {
            ui.label(format!(
                "{} points completed",
                self.progress.lock().unwrap().points
            ));
        }
        super::expression_message(ui, self.error.clone().map_or(Ok(()), Err));
        ui.separator();
        ui.label("Datasets");
        let mut delete = None;
        for (i, d) in self.datasets.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.visible, "Show");
                    ui.checkbox(&mut d.selected, "Export");
                    if ui.selectable_label(self.selected == i, "Table").clicked() {
                        self.selected = i;
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(i);
                    }
                });
                ui.add(crate::tokens::input(
                    egui::TextEdit::singleline(&mut d.label).char_limit(80),
                ));
                if d.partial {
                    ui.label("Stopped · partial dataset");
                }
            });
        }
        if let Some(i) = delete {
            self.datasets.remove(i);
            self.selected = self.selected.min(self.datasets.len().saturating_sub(1));
        }
    }
    fn main_caption(&self) -> &'static str {
        "ERROR RATE"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.ser_view, "Symbol error rate");
        let accent = crate::tokens::palette(ui.ctx()).accent;
        egui_plot::Plot::new("ber_plot")
            .x_axis_label("Eb/N0 (dB)")
            .y_axis_label(if self.ser_view { "SER" } else { "BER" })
            .y_axis_formatter(|mark, _| format!("{:.0e}", 10_f64.powf(mark.value)))
            .legend(egui_plot::Legend::default())
            .height(240.)
            .show(ui, |p| {
                let mut draw = |d: &Dataset| {
                    if !d.visible {
                        return;
                    }
                    let y = if self.ser_view { &d.ser } else { &d.ber };
                    let pts =
                        d.x.iter()
                            .zip(y)
                            .filter(|(_, y)| **y > 0.)
                            .map(|(&x, &y)| [x, y.log10()])
                            .collect::<Vec<_>>();
                    if d.spec.simulation {
                        p.points(egui_plot::Points::new(&d.label, pts).color(accent));
                    } else {
                        p.line(egui_plot::Line::new(&d.label, pts).color(accent));
                    }
                    if !self.ser_view && !d.fit.is_empty() {
                        p.line(
                            egui_plot::Line::new(
                                format!("{} fit", d.label),
                                d.fit
                                    .iter()
                                    .filter(|v| v[1] > 0.)
                                    .map(|v| [v[0], v[1].log10()])
                                    .collect::<Vec<_>>(),
                            )
                            .color(accent)
                            .style(egui_plot::LineStyle::dashed_dense()),
                        );
                    }
                };
                for d in &self.datasets {
                    draw(d);
                }
                if self.running
                    && let Some(d) = &self.progress.lock().unwrap().latest
                {
                    draw(d);
                }
            });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Datasets", self.datasets.len().to_string()),
            (
                "Selected for export",
                self.selected_sets().len().to_string(),
            ),
        ]
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if let Some(d) = self.datasets.get(self.selected) {
            egui::ScrollArea::vertical()
                .max_height(240.)
                .show_themed(ui, |ui| {
                    egui::Grid::new("ber_table").striped(true).show(ui, |ui| {
                        for h in ["Eb/N0", "BER", "Errors", "Bits"] {
                            ui.label(h);
                        }
                        ui.end_row();
                        for i in 0..d.x.len() {
                            for v in [d.x[i], d.ber[i], d.errors[i], d.bits[i]] {
                                ui.label(short(v));
                            }
                            ui.end_row();
                        }
                    });
                });
        }
        name_field(ui, "Results struct", &mut self.name, ws);
        super::expression_message(ui, checked_name(&self.name));
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        vec![self.name.clone()]
    }
    fn summary(&self) -> Option<String> {
        self.datasets.get(self.selected).map(|d| {
            format!(
                "{} · {} points{}",
                d.label,
                d.x.len(),
                if d.partial { " · partial" } else { "" }
            )
        })
    }
}
pub(crate) fn show(
    h: &mut Host<BerAnalysis>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    h.show(ui, ws, busy, action);
    if !h.running() && !h.app.launch_pending.load(Ordering::Relaxed) {
        h.app.running = false;
    }
}
