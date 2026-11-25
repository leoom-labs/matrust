//! Isolated, cancellable Filter Designer preview. One in-flight job designs the
//! filter from the spec's script in a private Engine, then computes every
//! analysis view in Rust, so switching tabs never recomputes.
use crate::filter_spec::{Coefficients, MaskBand, Spec, Structure, Units, sos_matrix};
use matrust::dsp::{self, Analysis};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// Frequency points of every frequency-domain view.
pub(super) const POINTS: usize = 1024;

/// Analysis views, in tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum View {
    Magnitude,
    Phase,
    MagnitudePhase,
    GroupDelay,
    PhaseDelay,
    Impulse,
    Step,
    PoleZero,
    Coefficients,
    Info,
}
impl View {
    pub(super) const ALL: [Self; 10] = [
        Self::Magnitude,
        Self::Phase,
        Self::MagnitudePhase,
        Self::GroupDelay,
        Self::PhaseDelay,
        Self::Impulse,
        Self::Step,
        Self::PoleZero,
        Self::Coefficients,
        Self::Info,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Magnitude => "Magnitude",
            Self::Phase => "Phase",
            Self::MagnitudePhase => "Mag + phase",
            Self::GroupDelay => "Group delay",
            Self::PhaseDelay => "Phase delay",
            Self::Impulse => "Impulse",
            Self::Step => "Step",
            Self::PoleZero => "Pole-zero",
            Self::Coefficients => "Coefficients",
            Self::Info => "Info",
        }
    }
}

/// Everything the analysis area draws, computed once per design.
#[derive(Clone, Debug)]
pub(super) struct Preview {
    pub coefficients: Coefficients,
    /// Frequency axis in display units (Hz or ×π rad/sample).
    pub freq: Vec<f64>,
    pub freq_label: &'static str,
    /// Display units per normalized frequency unit (Nyquist = 1).
    pub freq_scale: f64,
    pub magnitude_db: Vec<f64>,
    /// Unwrapped phase, degrees.
    pub phase: Vec<f64>,
    /// Samples.
    pub group_delay: Vec<f64>,
    pub phase_delay: Vec<f64>,
    pub impulse: Vec<f64>,
    pub step: Vec<f64>,
    pub zeros: Vec<[f64; 2]>,
    pub poles: Vec<[f64; 2]>,
    pub stable: bool,
    pub order: usize,
    pub mask: Vec<MaskBand>,
    pub title: String,
    pub structure: &'static str,
}

struct Pending {
    key: String,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<matrust::Result<Preview>>,
    thread: std::thread::JoinHandle<()>,
}
#[derive(Default)]
pub(super) struct Cache {
    key: Option<String>,
    pending: Option<Pending>,
    result: Option<matrust::Result<Preview>>,
}
impl Cache {
    /// Start (or keep) the preview for `spec`; an invalid spec clears it.
    pub(super) fn refresh(&mut self, ctx: &eframe::egui::Context, spec: &Spec) {
        let key = match spec.script() {
            Ok(script) => script,
            Err(error) => {
                self.cancel();
                self.key = None;
                self.result = Some(Err(error));
                return;
            }
        };
        if self.key.as_ref() != Some(&key) {
            self.cancel();
            self.key = Some(key.clone());
            self.result = None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.thread.is_finished())
        {
            let pending = self.pending.take().unwrap();
            if pending.key == key {
                self.result = Some(
                    pending
                        .receive
                        .try_recv()
                        .unwrap_or_else(|_| Err("Preview worker stopped".into())),
                );
            }
            let _ = pending.thread.join();
        }
        if self.pending.is_none() && self.result.is_none() {
            let mut engine = matrust::Engine::default();
            let cancel = engine.cancellation();
            let (send, receive) = mpsc::sync_channel(1);
            let context = ctx.clone();
            let spec = spec.clone();
            let thread = std::thread::spawn(move || {
                let _ = send.send(prepare(&mut engine, &spec));
                context.request_repaint();
            });
            self.pending = Some(Pending {
                key,
                cancel,
                receive,
                thread,
            });
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    pub(super) fn cancel(&self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Relaxed);
        }
    }
    /// None while the first design is computing.
    pub(super) fn result(&self) -> Option<&matrust::Result<Preview>> {
        self.result.as_ref()
    }
    pub(super) fn preview(&self) -> Option<&Preview> {
        self.result.as_ref()?.as_ref().ok()
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        self.cancel();
        if let Some(pending) = self.pending.take() {
            let _ = pending.thread.join();
        }
    }
}

/// Impulse/step length: FIR = its taps; IIR = until the slowest pole decays
/// to 1e-5 (32..4096); unstable = 128.
fn time_length(coefficients: &Coefficients, poles: &[[f64; 2]], stable: bool) -> usize {
    if let Coefficients::Tf { a, .. } = coefficients
        && a.iter().skip(1).all(|v| *v == 0.)
    {
        let Coefficients::Tf { b, .. } = coefficients else {
            unreachable!()
        };
        return b.len().clamp(2, 4096);
    }
    if !stable {
        return 128;
    }
    let r = poles.iter().map(|p| p[0].hypot(p[1])).fold(0., f64::max);
    if r <= 0. {
        return 32;
    }
    ((1e-5f64.ln() / r.ln()).ceil() as usize).clamp(32, 4096)
}

pub(super) fn prepare(engine: &mut matrust::Engine, spec: &Spec) -> matrust::Result<Preview> {
    let coefficients = crate::filter_spec::design(spec, engine)?;
    let cancel = engine.cancellation();
    let mut f: Vec<f64> = (0..POINTS).map(|i| i as f64 / POINTS as f64).collect();
    // The notch frequency itself is on the grid, so the plot shows its depth.
    if spec.response == crate::filter_spec::Response::Notch {
        let f0 = spec.norm(spec.f0);
        if f0 > 0. && f0 < 1. {
            let at = f.partition_point(|v| *v < f0);
            if f[at.min(POINTS - 1)] != f0 {
                f.insert(at, f0);
            }
        }
    }
    let h = coefficients.response(&f, &cancel)?;
    let magnitude_db = h
        .iter()
        .map(|[r, i]| 20. * r.hypot(*i).max(1e-15).log10())
        .collect();
    let phase_rad = dsp::phase_response(&h);
    let phase = phase_rad.iter().map(|p| p.to_degrees()).collect();
    // Roots of a very long FIR are out of reach; the other views still show.
    let (group_delay, zpk) = match &coefficients {
        Coefficients::Tf { b, a } => (
            dsp::group_delay(b, a, &f, &cancel)?,
            dsp::iir::tf2zpk(b, a).ok(),
        ),
        Coefficients::Sos { sos, .. } => {
            let m = sos_matrix(sos)?;
            (
                dsp::sos_group_delay(&m, &f, &cancel)?,
                dsp::iir::sos2zpk(&m).ok(),
            )
        }
    };
    let (zeros, mut poles): (Vec<[f64; 2]>, Vec<[f64; 2]>) =
        zpk.map_or_else(Default::default, |zpk| {
            let roots = |v: Vec<_>| {
                v.into_iter()
                    .map(|z: nalgebra::Complex<f64>| [z.re, z.im])
                    .collect()
            };
            (roots(zpk.zeros), roots(zpk.poles))
        });
    // DC phase delay is the group delay there, the same fill phasedelay uses.
    let phase_delay = phase_rad
        .iter()
        .zip(&f)
        .enumerate()
        .map(|(i, (p, fr))| {
            if i == 0 {
                group_delay[0]
            } else {
                -p / (std::f64::consts::PI * fr)
            }
        })
        .collect();
    // A causal FIR (or any proper filter) has its remaining poles at z = 0.
    poles.resize(poles.len().max(zeros.len()), [0., 0.]);
    let stable = poles.iter().all(|p| p[0].hypot(p[1]) < 1. - 1e-12);
    let n = time_length(&coefficients, &poles, stable);
    let time = |kind| -> matrust::Result<Vec<f64>> {
        match &coefficients {
            Coefficients::Tf { b, a } => dsp::analyze(kind, b, a, n, &cancel),
            Coefficients::Sos { sos, g } => {
                Ok(dsp::analyze_sos(kind, &sos_matrix(sos)?, n, &cancel)?
                    .into_iter()
                    .map(|v| v * g)
                    .collect())
            }
        }
    };
    let (impulse, step) = (time(Analysis::Impulse)?, time(Analysis::Step)?);
    let (freq_scale, freq_label) = match spec.units {
        Units::Hz => (spec.fs / 2., "Frequency (Hz)"),
        Units::Normalized => (1., "Normalized frequency (×π rad/sample)"),
    };
    Ok(Preview {
        order: coefficients.order(),
        freq: f.iter().map(|v| v * freq_scale).collect(),
        freq_label,
        freq_scale,
        magnitude_db,
        phase,
        group_delay,
        phase_delay,
        impulse,
        step,
        zeros,
        poles,
        stable,
        mask: spec.mask(),
        title: spec.title(),
        structure: match (&coefficients, spec.structure) {
            (Coefficients::Sos { .. }, _) => "Second-order sections",
            (_, Structure::Sos) if spec.method.iir() => "Second-order sections",
            _ => "Direct form",
        },
        coefficients,
    })
}
