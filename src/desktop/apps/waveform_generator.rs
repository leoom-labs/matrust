//! Seeded, script-first wireless waveform laboratory. No numerical UI kernels.
use super::ber_analysis::{
    Modulation, checked_name, modem_calls, modulation_ui, numbers, numeric_text, vector,
};
use super::{
    App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, number, short,
};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::Array;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pulse {
    None,
    Raised,
    Root,
}
#[derive(Clone, Debug)]
pub struct Spec {
    pub modulation: Modulation,
    pub ofdm: bool,
    pub order: u32,
    pub symbols: u32,
    pub random: bool,
    pub bits_name: String,
    pub seed: u32,
    pub sps: u32,
    pub pulse: Pulse,
    pub beta: f64,
    pub span: u32,
    pub sample_rate: f64,
    pub nfft: u32,
    pub cp: u32,
    pub nulls: String,
    pub pilots: String,
    pub noise: bool,
    pub snr: f64,
    pub frequency: f64,
    pub phase: f64,
    pub dc_i: f64,
    pub dc_q: f64,
    pub iq_gain: f64,
    pub iq_phase: f64,
    pub captured_bits: Option<Arc<Vec<f64>>>,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            modulation: Modulation::Psk,
            ofdm: false,
            order: 4,
            symbols: 256,
            random: true,
            bits_name: "bits".into(),
            seed: 7,
            sps: 4,
            pulse: Pulse::None,
            beta: 0.25,
            span: 10,
            sample_rate: 1_000_000.,
            nfft: 64,
            cp: 16,
            nulls: "[1 33]".into(),
            pilots: "[]".into(),
            noise: false,
            snr: 20.,
            frequency: 0.,
            phase: 0.,
            dc_i: 0.,
            dc_q: 0.,
            iq_gain: 0.,
            iq_phase: 0.,
            captured_bits: None,
        }
    }
}
impl Spec {
    fn indices(&self) -> Result<(Vec<f64>, Vec<f64>), String> {
        let null = numeric_text(&self.nulls, 256)?;
        let pilot = numeric_text(&self.pilots, 256)?;
        let mut all = Vec::new();
        for &i in null.iter().chain(&pilot) {
            if i.fract() != 0. || i < 1. || i > self.nfft as f64 || all.contains(&i) {
                return Err("Null and pilot indices must be distinct integers in 1…nfft".into());
            }
            all.push(i);
        }
        if all.len() >= self.nfft as usize {
            return Err("OFDM needs at least one data subcarrier".into());
        }
        Ok((null, pilot))
    }
    pub fn validate(&self) -> Result<(), String> {
        if !Modulation::SIM.contains(&self.modulation)
            || self.modulation == Modulation::Dpsk
            || !self.modulation.order_ok(self.order)
            || !(1..=65536).contains(&self.symbols)
            || !(1..=2048).contains(&self.sps)
            || !(2..=64).contains(&self.span)
            || !(8..=256).contains(&self.nfft)
            || !self.nfft.is_power_of_two()
            || self.cp > self.nfft
        {
            return Err("Choose supported modulation, 1–65536 symbols, 1–2048 samples/symbol, even span 2–64, and FFT 8–256".into());
        }
        if self.ofdm && !matches!(self.modulation, Modulation::Psk | Modulation::Qam) {
            return Err("OFDM uses PSK or QAM sub-modulation".into());
        }
        if matches!(self.modulation, Modulation::Fsk | Modulation::Msk)
            && (self.sps < 2 || (self.modulation == Modulation::Fsk && self.sps < 2 * self.order))
        {
            return Err("FSK needs at least 2M samples/symbol; MSK at least 2".into());
        }
        if !self.span.is_multiple_of(2)
            || !self.beta.is_finite()
            || !(0.0..=1.0).contains(&self.beta)
            || !self.sample_rate.is_finite()
            || !(1.0..=1e12).contains(&self.sample_rate)
        {
            return Err("Use even pulse span, rolloff 0–1 and sample rate 1…1e12 Hz".into());
        }
        for (v, lo, hi, label) in [
            (self.snr, -100., 100., "SNR"),
            (
                self.frequency,
                -self.sample_rate / 2.,
                self.sample_rate / 2.,
                "Frequency offset",
            ),
            (self.phase, -360., 360., "Phase offset"),
            (self.dc_i, -10., 10., "DC I"),
            (self.dc_q, -10., 10., "DC Q"),
            (self.iq_gain, -20., 20., "IQ gain"),
            (self.iq_phase, -89., 89., "IQ phase"),
        ] {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{label} must be finite in {lo}…{hi}"));
            }
        }
        let samples = if self.ofdm {
            (self.nfft + self.cp) as u64 * self.symbols as u64 * self.sps as u64
                + if self.pulse == Pulse::None {
                    0
                } else {
                    self.span as u64 * self.sps as u64
                }
        } else {
            (self.symbols + self.span) as u64 * self.sps as u64
        };
        if samples > 1_048_576 {
            return Err(
                "Waveform exceeds 1048576 samples; reduce symbols, FFT size or oversampling".into(),
            );
        }
        if self.ofdm {
            self.indices()?;
        }
        Ok(())
    }
    fn bit_count(&self) -> Result<usize, String> {
        let carriers = if self.ofdm {
            let (n, p) = self.indices()?;
            self.nfft as usize - n.len() - p.len()
        } else {
            1
        };
        Ok(carriers * self.symbols as usize * self.order.ilog2() as usize)
    }
    pub fn capture(&self, ws: &Snapshot) -> Result<Self, String> {
        self.validate()?;
        let mut s = self.clone();
        if !self.random {
            checked_name(&self.bits_name)?;
            let a = ws
                .get(&self.bits_name)
                .ok_or("Choose an existing workspace bit vector")?;
            if a.rows() > 1 && a.cols() > 1 {
                return Err("Workspace bits must be a vector".into());
            }
            let count = self.bit_count()?;
            if a.len() != count || a.len() > 131072 {
                return Err(format!(
                    "Provide exactly {count} bits (workspace source limit 131072)"
                ));
            }
            let bits = match a {
                Array::Numeric(m) => m.data().to_vec(),
                Array::Logical(m) => m.data().iter().map(|&b| f64::from(b)).collect(),
                Array::Typed(m) => (0..m.len()).map(|i| m.real(i)).collect(),
                _ => return Err("Workspace bits must be real or logical zeros and ones".into()),
            };
            let count = self.bit_count()?;
            if bits.len() != count
                || bits.iter().any(|&b| b != 0. && b != 1.)
                || bits.len() > 131072
            {
                return Err(format!(
                    "Provide exactly {count} binary values (workspace source limit 131072)"
                ));
            }
            s.captured_bits = Some(Arc::new(bits));
        }
        Ok(s)
    }
}
/// Single builder used unchanged for preview, export and script save. Captured
/// workspace bits become literals so edits cannot invalidate a saved waveform.
fn col_vector(v: &[f64]) -> String {
    format!(
        "[{}]",
        v.iter().map(|v| number(*v)).collect::<Vec<_>>().join(";")
    )
}
pub fn build_script(s: &Spec, names: &[String; 2]) -> Result<String, String> {
    s.validate()?;
    for n in names {
        checked_name(n)?;
    }
    if names[0] == names[1] {
        return Err("Waveform and settings names must differ".into());
    }
    let count = s.bit_count()?;
    let source = if s.random {
        format!("randi([0 1],{count},1)")
    } else {
        let b = s
            .captured_bits
            .as_ref()
            .ok_or("Capture workspace bits with Generate first")?;
        if b.len() != count || b.iter().any(|&v| v != 0. && v != 1.) {
            return Err("Invalid captured bits".into());
        }
        format!("{} .'", vector(b))
    };
    let mut code = format!(
        "[{},{}]=app_comm_wave();\nfunction [w,settings]=app_comm_wave()\nrng({});\nb={source};\nd=bit2int(b,{});\n",
        names[0],
        names[1],
        s.seed,
        s.order.ilog2()
    );
    let m = s.order;
    let ns = s.sps;
    if s.ofdm {
        let (null, pilot) = s.indices()?;
        let carriers = s.nfft as usize - null.len() - pilot.len();
        let md = modem_calls(s.modulation, m, 1, "d", "z").0;
        code += &format!(
            "reference={md};\ndata=reshape(reference,{carriers},{});\nw=ofdmmod(data,{},{},{},{},ones({},{}),'OversamplingFactor',{ns});\nh=1;\n",
            s.symbols,
            s.nfft,
            s.cp,
            col_vector(&null),
            col_vector(&pilot),
            pilot.len(),
            s.symbols
        );
    } else {
        let md = modem_calls(s.modulation, m, ns, "d", "z").0;
        code += &format!("reference={md};\n");
        if matches!(s.modulation, Modulation::Fsk | Modulation::Msk) {
            code += "h=1; w=reference;\n";
        } else if s.pulse == Pulse::None {
            code += &format!(
                "h=ones({ns},1); w=complex(upfirdn(real(reference),h,{ns},1),upfirdn(imag(reference),h,{ns},1));\n"
            );
        } else {
            code += &format!(
                "h=rcosdesign({},{},{ns},'{}');\nw=complex(upfirdn(real(reference),h,{ns},1),upfirdn(imag(reference),h,{ns},1));\n",
                number(s.beta),
                s.span,
                if s.pulse == Pulse::Root {
                    "sqrt"
                } else {
                    "normal"
                }
            );
        }
    }
    // These modems already return sampled waveforms. Filter at the existing
    // sample rate; linear constellations above combine interpolation/filtering.
    if s.pulse != Pulse::None
        && (s.ofdm || matches!(s.modulation, Modulation::Fsk | Modulation::Msk))
    {
        code += &format!(
            "h=rcosdesign({},{},{ns},'{}');\nw=complex(upfirdn(real(w),h,1,1),upfirdn(imag(w),h,1,1));\n",
            number(s.beta),
            s.span,
            if s.pulse == Pulse::Root {
                "sqrt"
            } else {
                "normal"
            }
        );
    }
    code += &format!(
        "w=w(:);\nt=(0:numel(w)-1)'/{};\ng=10^({}/40);\ni=real(w)*g; q=imag(w)/g;\nw=i+1i*(q*cos({}*pi/180)+i*sin({}*pi/180));\nw=w.*exp(1i*(2*pi*{}*t+{}*pi/180))+complex({},{});\n",
        number(s.sample_rate),
        number(s.iq_gain),
        number(s.iq_phase),
        number(s.iq_phase),
        number(s.frequency),
        number(s.phase),
        number(s.dc_i),
        number(s.dc_q)
    );
    code += "w=complex(w);\n";
    if s.noise {
        code += &format!("w=awgn(w,{},'measured');\n", number(s.snr));
    }
    code += &format!(
        "w=complex(w);\npower=mean(abs(w).^2); papr=10*log10(max(abs(w).^2)/power);\nsettings=struct('Type','{}','Order',{m},'Symbols',{},'Seed',{},'Random',{},'SamplesPerSymbol',{ns},'SampleRate',{},'Pulse','{:?}','Rolloff',{},'Span',{},'NFFT',{},'CP',{},'Nulls',{},'Pilots',{},'AWGN',{},'SNR',{},'FrequencyOffset',{},'PhaseOffset',{},'DC',complex({},{}),'IQGain',{},'IQPhase',{},'Length',numel(w),'Power',power,'PAPR',papr,'ReferenceSymbols',reference,'Filter',h);\n",
        if s.ofdm { "OFDM" } else { s.modulation.label() },
        s.symbols,
        s.seed,
        u8::from(s.random),
        number(s.sample_rate),
        s.pulse,
        number(s.beta),
        s.span,
        s.nfft,
        s.cp,
        if s.ofdm {
            vector(&s.indices()?.0)
        } else {
            "[]".into()
        },
        if s.ofdm {
            vector(&s.indices()?.1)
        } else {
            "[]".into()
        },
        u8::from(s.noise),
        number(s.snr),
        number(s.frequency),
        number(s.phase),
        number(s.dc_i),
        number(s.dc_q),
        number(s.iq_gain),
        number(s.iq_phase)
    );
    code += &format!(
        "settings.Submodulation='{}'; settings.SourceBits=b;\n",
        s.modulation.label()
    );
    // Limit plot data independently of the exported waveform size.
    code += "figure; v=1:min(4096,numel(w)); plot(t(v),real(w(v))); hold on; plot(t(v),imag(w(v))); title('Time'); xlabel('Time (s)'); ylabel('Amplitude');\n";
    code += &format!(
        "figure; n=min(256,numel(w)); [p,f]=pwelch(w,hamming(n),floor(n/2),512,{},'centered'); plot(f,10*log10(max(p,1e-30))); title('Spectrum'); xlabel('Frequency (Hz)'); ylabel('Power density (dB/Hz)');\n",
        number(s.sample_rate)
    );
    if s.ofdm {
        let (n, p) = s.indices()?;
        let input = if s.pulse == Pulse::None {
            "w"
        } else {
            let delay = if s.pulse == Pulse::Root {
                code += "view=complex(upfirdn(real(w),h,1,1),upfirdn(imag(w),h,1,1));\n";
                s.span * ns
            } else {
                code += "view=w;\n";
                s.span * ns / 2
            };
            let length = (s.nfft + s.cp) * s.symbols * ns;
            code += &format!("view=view({}+1:{}+{});\n", delay, delay, length);
            "view"
        };
        code += &format!(
            "c=ofdmdemod({input},{},{},{},{},{},'OversamplingFactor',{ns}); c=c(:);\n",
            s.nfft,
            s.cp,
            s.cp,
            col_vector(&n),
            col_vector(&p)
        );
    } else if s.pulse == Pulse::Raised {
        code += &format!(
            "c=w({}+1:{ns}:{}+{}*{ns});\n",
            s.span * ns / 2,
            s.span * ns / 2,
            s.symbols
        );
    } else if s.pulse == Pulse::Root {
        code += &format!(
            "c=complex(upfirdn(real(w),h,1,{ns}),upfirdn(imag(w),h,1,{ns})); c=c({}+1:{}+{});\n",
            s.span, s.span, s.symbols
        );
    } else {
        code += &format!("c=w(1:{ns}:end);\n");
    }
    code += "scatterplot(c(1:min(8192,numel(c)))); title('Constellation');\n";
    code += &format!(
        "eyediagram(w(1:min(numel(w),{})),{},{});\nend\n",
        128 * ns,
        2 * ns,
        number(2. * ns as f64 / s.sample_rate)
    );
    if code.len() > 900 * 1024 {
        return Err("Waveform script exceeds 900 KiB".into());
    }
    Ok(code)
}
#[derive(Clone)]
pub struct Chart {
    pub title: String,
    pub x: String,
    pub y: String,
    pub series: Vec<Vec<[f64; 2]>>,
}
#[derive(Clone)]
pub struct ResultView {
    pub waveform: Array,
    pub power: f64,
    pub papr: f64,
    pub charts: Vec<Chart>,
    pub spec: Spec,
}
pub struct WaveformGenerator {
    pub spec: Spec,
    pub names: [String; 2],
    pub current: Option<ResultView>,
    pub view: usize,
    pub error: Option<String>,
    active: Option<Spec>,
    pending: Arc<Mutex<Option<ResultView>>>,
    serial: u64,
    pub running: bool,
    launch_pending: AtomicBool,
    pub stop_requested: bool,
}
impl Default for WaveformGenerator {
    fn default() -> Self {
        Self {
            spec: Spec::default(),
            names: ["waveform".into(), "waveform_settings".into()],
            current: None,
            view: 0,
            error: None,
            active: None,
            pending: Arc::default(),
            serial: 0,
            running: false,
            launch_pending: AtomicBool::new(false),
            stop_requested: false,
        }
    }
}
impl WaveformGenerator {
    pub fn start(&mut self, ws: &Snapshot) -> Result<(), String> {
        let s = self.spec.capture(ws)?;
        build_script(&s, &self.names)?;
        self.active = Some(s);
        self.serial += 1;
        *self.pending.lock().unwrap() = None;
        self.running = true;
        self.launch_pending.store(true, Ordering::Relaxed);
        self.error = None;
        Ok(())
    }
}
impl App for WaveformGenerator {
    fn kind(&self) -> AppKind {
        AppKind::WirelessWaveformGenerator
    }
    fn script(&self, _: &Snapshot) -> Result<String, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        build_script(self.active.as_ref().unwrap_or(&self.spec), &self.names)
    }
    fn export_code(&self, _: &Snapshot) -> Result<String, String> {
        let r = self
            .current
            .as_ref()
            .ok_or("Generate a waveform before exporting")?;
        build_script(&r.spec, &self.names)
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (
            format!(
                "app_comm_metrics=[{}.Power {}.PAPR];\n",
                self.names[1], self.names[1]
            ),
            vec![self.names[0].clone(), "app_comm_metrics".into()],
        )
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, request: Request) {
        self.launch_pending.store(false, Ordering::Relaxed);
        if !self.running || runner.key() == Some(self.serial) {
            runner.poll(ctx);
            return;
        }
        let pending = self.pending.clone();
        let spec = self.active.clone().unwrap();
        let name = self.names[0].clone();
        runner.submit_task(
            ctx,
            self.serial,
            Box::new(move |e| {
                let o = super::run(e, request)?;
                let metrics = numbers(&o, "app_comm_metrics")?;
                let mut figures = e.figures().collect::<Vec<_>>();
                figures.sort_by_key(|(i, _)| *i);
                let charts = figures
                    .into_iter()
                    .flat_map(|(_, f)| {
                        f.all_axes()
                            .map(|(_, a)| Chart {
                                title: a.title.clone(),
                                x: a.xlabel.clone(),
                                y: a.ylabel.clone(),
                                series: a.series.iter().map(|s| s.display_points(8192)).collect(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                *pending.lock().map_err(|_| "Waveform result lock failed")? = Some(ResultView {
                    waveform: o.get(&name).ok_or("Missing waveform")?.clone(),
                    power: metrics[0],
                    papr: metrics[1],
                    charts,
                    spec,
                });
                Ok(o)
            }),
        );
    }
    fn receive(&mut self, _: &Outputs) -> Result<(), String> {
        self.running = false;
        self.current = self
            .pending
            .lock()
            .map_err(|_| "Waveform result lock failed")?
            .take()
            .or(self.current.take());
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        if ui.checkbox(&mut self.spec.ofdm, "OFDM").changed()
            && self.spec.ofdm
            && !matches!(self.spec.modulation, Modulation::Psk | Modulation::Qam)
        {
            self.spec.modulation = Modulation::Psk;
            self.spec.order = 4;
        }
        modulation_ui(
            ui,
            "wave_mod",
            &mut self.spec.modulation,
            if self.spec.ofdm {
                &[Modulation::Psk, Modulation::Qam]
            } else {
                &[
                    Modulation::Psk,
                    Modulation::Qam,
                    Modulation::Pam,
                    Modulation::Fsk,
                    Modulation::Msk,
                ]
            },
            &mut self.spec.order,
            1024,
        );
        field(
            ui,
            if self.spec.ofdm {
                "OFDM symbols"
            } else {
                "Symbols"
            },
            |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.symbols).range(1..=65536));
            },
        );
        ui.checkbox(&mut self.spec.random, "Random bits");
        if self.spec.random {
            field(ui, "Seed", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.seed));
            });
        } else {
            name_field(ui, "Workspace bit vector", &mut self.spec.bits_name, ws);
        }
        field(ui, "Samples per symbol / oversampling", |ui| {
            ui.add(egui::DragValue::new(&mut self.spec.sps).range(1..=2048));
        });
        if self.spec.ofdm {
            field(ui, "FFT length", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.nfft).range(8..=256));
            });
            field(ui, "Cyclic prefix", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.cp).range(0..=256));
            });
            field(ui, "Null indices (centered, 1-based)", |ui| {
                ui.add(crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.spec.nulls).char_limit(2048),
                ));
            });
            field(ui, "Pilot indices (unit pilots)", |ui| {
                ui.add(crate::tokens::input(
                    egui::TextEdit::singleline(&mut self.spec.pilots).char_limit(2048),
                ));
            });
        }
        egui::ComboBox::from_id_salt("pulse")
            .selected_text(match self.spec.pulse {
                Pulse::None => "None",
                Pulse::Raised => "Raised cosine",
                Pulse::Root => "Root raised cosine",
            })
            .show_ui(ui, |ui| {
                for (p, l) in [
                    (Pulse::None, "None"),
                    (Pulse::Raised, "Raised cosine"),
                    (Pulse::Root, "Root raised cosine"),
                ] {
                    ui.selectable_value(&mut self.spec.pulse, p, l);
                }
            });
        if self.spec.pulse != Pulse::None {
            field(ui, "Rolloff", |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.spec.beta)
                        .range(0.0..=1.0)
                        .speed(0.01),
                );
            });
            field(ui, "Span (symbols, even)", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.span).range(2..=64));
            });
        }
        field(ui, "Sample rate (Hz)", |ui| {
            ui.add(egui::DragValue::new(&mut self.spec.sample_rate).range(1.0..=1e12));
        });
        ui.separator();
        ui.label("Impairments");
        ui.checkbox(&mut self.spec.noise, "AWGN");
        if self.spec.noise {
            field(ui, "SNR (dB)", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.snr).range(-100.0..=100.0));
            });
        }
        for (label, v, lo, hi) in [
            (
                "Frequency offset (Hz)",
                &mut self.spec.frequency,
                -self.spec.sample_rate / 2.,
                self.spec.sample_rate / 2.,
            ),
            ("Phase offset (degrees)", &mut self.spec.phase, -360., 360.),
            ("DC I", &mut self.spec.dc_i, -10., 10.),
            ("DC Q", &mut self.spec.dc_q, -10., 10.),
            ("IQ gain imbalance (dB)", &mut self.spec.iq_gain, -20., 20.),
            (
                "IQ phase imbalance (degrees)",
                &mut self.spec.iq_phase,
                -89.,
                89.,
            ),
        ] {
            field(ui, label, |ui| {
                ui.add(egui::DragValue::new(v).range(lo..=hi).speed(0.1));
            });
        }
        super::expression_message(ui, self.spec.validate());
        if ui
            .add_enabled(!self.running, egui::Button::new("Generate waveform"))
            .clicked()
        {
            self.error = self.start(ws).err();
            if self.running {
                ui.ctx().request_repaint();
            }
        }
        if ui
            .add_enabled(self.running, egui::Button::new("Stop"))
            .clicked()
        {
            self.stop_requested = true;
        }
        super::expression_message(ui, self.error.clone().map_or(Ok(()), Err));
    }
    fn main_caption(&self) -> &'static str {
        "WAVEFORM"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (i, s) in ["Time", "Spectrum", "Constellation", "Eye"]
                .iter()
                .enumerate()
            {
                ui.selectable_value(&mut self.view, i, *s);
            }
        });
        let Some(r) = &self.current else {
            ui.label("Generate a waveform to inspect it.");
            return;
        };
        let indices: Vec<usize> = match self.view {
            0 => vec![0],
            1 => vec![1],
            2 => vec![2],
            _ => vec![3, 4],
        };
        let accent = crate::tokens::palette(ui.ctx()).accent;
        egui::ScrollArea::vertical().show_themed(ui, |ui| {
            for i in indices {
                if let Some(c) = r.charts.get(i) {
                    ui.label(&c.title);
                    egui_plot::Plot::new(("wave_plot", i))
                        .height(220.)
                        .x_axis_label(&c.x)
                        .y_axis_label(&c.y)
                        .show(ui, |p| {
                            for (j, s) in c.series.iter().enumerate() {
                                let label = if j == 0 { "I" } else { "Q" };
                                if i == 2 {
                                    p.points(
                                        egui_plot::Points::new(label, s.clone())
                                            .color(accent)
                                            .radius(4.),
                                    );
                                } else {
                                    p.line(
                                        egui_plot::Line::new(label, s.clone()).color(accent).style(
                                            if j == 0 {
                                                egui_plot::LineStyle::Solid
                                            } else {
                                                egui_plot::LineStyle::dashed_dense()
                                            },
                                        ),
                                    );
                                }
                            }
                        });
                }
            }
        });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        self.current
            .as_ref()
            .map(|r| {
                vec![
                    ("Length", r.waveform.len().to_string()),
                    ("Sample rate (Hz)", short(r.spec.sample_rate)),
                    ("Measured power", short(r.power)),
                    ("PAPR (dB)", short(r.papr)),
                ]
            })
            .unwrap_or_default()
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        name_field(ui, "Waveform", &mut self.names[0], ws);
        name_field(ui, "Settings", &mut self.names[1], ws);
        for n in &self.names {
            super::expression_message(ui, checked_name(n));
        }
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        self.names.to_vec()
    }
    fn summary(&self) -> Option<String> {
        self.current
            .as_ref()
            .map(|r| format!("{} samples · PAPR {} dB", r.waveform.len(), short(r.papr)))
    }
}
pub(crate) fn show(
    h: &mut Host<WaveformGenerator>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    h.show(ui, ws, busy, action);
    if std::mem::take(&mut h.app.stop_requested) {
        h.runner.stop();
        h.app.launch_pending.store(false, Ordering::Relaxed);
        h.app.running = false;
    }
    if !h.running() && !h.app.launch_pending.load(Ordering::Relaxed) {
        h.app.running = false;
    }
}
