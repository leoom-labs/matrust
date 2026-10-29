//! Bounded filter analysis, using analytic group delay rather than differences of phase.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Analysis {
    #[default]
    Magnitude,
    Phase,
    GroupDelay,
    Impulse,
    Step,
}
impl Analysis {
    pub const ALL: [Self; 5] = [
        Self::Magnitude,
        Self::Phase,
        Self::GroupDelay,
        Self::Impulse,
        Self::Step,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Magnitude => "Magnitude (dB)",
            Self::Phase => "Phase (rad)",
            Self::GroupDelay => "Group delay (samples)",
            Self::Impulse => "Impulse response",
            Self::Step => "Step response",
        }
    }
    pub fn time_domain(self) -> bool {
        matches!(self, Self::Impulse | Self::Step)
    }
    pub fn function(self, sos: bool) -> &'static str {
        match (self, sos) {
            (Self::Magnitude, false) => "freqzmag",
            (Self::Magnitude, true) => "sosfreqzmag",
            (Self::Phase, false) => "phasez",
            (Self::Phase, true) => "sosphasez",
            (Self::GroupDelay, false) => "grpdelay",
            (Self::GroupDelay, true) => "sosgrpdelay",
            (Self::Impulse, false) => "impz",
            (Self::Impulse, true) => "sosimpz",
            (Self::Step, false) => "stepz",
            (Self::Step, true) => "sosstepz",
        }
    }
    pub fn from_function(name: &str) -> Option<(Self, bool)> {
        Self::ALL.into_iter().find_map(|kind| {
            [false, true]
                .into_iter()
                .find_map(|sos| (kind.function(sos) == name).then_some((kind, sos)))
        })
    }
}

fn count(n: usize, cost: usize, cancel: &AtomicBool) -> Result<()> {
    check_cancel(cancel)?;
    if n == 0 || n > 65536 {
        return Err("Analysis point count must be 1..65536".into());
    }
    if n.saturating_mul(cost) > MAX_WORK {
        return Err("Filter analysis exceeds work limit".into());
    }
    Ok(())
}
fn frequency_grid(n: usize) -> Vec<f64> {
    (0..n).map(|i| i as f64 / n as f64).collect()
}

/// Phase in radians, unwrapped over each finite, nonzero response segment.
/// A zero or undefined response is represented by NaN and restarts unwrapping.
pub fn phase_response(response: &[[f64; 2]]) -> Vec<f64> {
    let mut previous = None;
    let mut offset = 0.;
    response
        .iter()
        .map(|&[re, im]| {
            if !re.is_finite() || !im.is_finite() || (re == 0. && im == 0.) {
                previous = None;
                offset = 0.;
                return f64::NAN;
            }
            let phase = im.atan2(re);
            if let Some(last) = previous {
                let delta: f64 = phase - last;
                if delta.abs() > PI {
                    offset -= std::f64::consts::TAU * (delta / std::f64::consts::TAU).round();
                }
            }
            previous = Some(phase);
            phase + offset
        })
        .collect()
}

/// Analytic group delay, in samples. Near polynomial zeros the result is undefined
/// and returned as NaN (rather than a misleading zero or an unbounded spike).
/// Coefficients are scaled before Horner evaluation to avoid scale-dependent overflow.
pub fn group_delay(b: &[f64], a: &[f64], f: &[f64], cancel: &AtomicBool) -> Result<Vec<f64>> {
    Filter::new(b, a)?;
    count(f.len(), 2 * (b.len() + a.len()), cancel)?;
    if f.iter().any(|v| !v.is_finite() || !(0. ..=1.).contains(v)) {
        return Err("Response frequencies must be between 0 and 1 (Nyquist)".into());
    }
    let scaled = |c: &[f64]| {
        let scale = c.iter().map(|v| v.abs()).fold(0., f64::max);
        let values: Vec<_> = c
            .iter()
            .map(|v| if scale == 0. { 0. } else { v / scale })
            .collect();
        let tolerance = 16. * f64::EPSILON * values.iter().map(|v| v.abs()).sum::<f64>();
        (values, tolerance)
    };
    let (b, btol) = scaled(b);
    let (a, atol) = scaled(a);
    let evaluate = |frequency: f64| {
        let (zi, zr) = (-PI * frequency).sin_cos();
        let logarithmic = |coeff: &[f64], tolerance: f64| {
            let (mut pr, mut pi, mut dr, mut di) = (0., 0., 0., 0.);
            for (k, &v) in coeff.iter().enumerate().rev() {
                (pr, pi) = (v + pr * zr - pi * zi, pr * zi + pi * zr);
                (dr, di) = (k as f64 * v + dr * zr - di * zi, dr * zi + di * zr);
            }
            if pr.hypot(pi) <= tolerance {
                return f64::NAN;
            }
            if pr.abs() >= pi.abs() {
                let r = pi / pr;
                (dr + di * r) / (pr + pi * r)
            } else {
                let r = pr / pi;
                (dr * r + di) / (pi + pr * r)
            }
        };
        logarithmic(&b, btol) - logarithmic(&a, atol)
    };
    let mut output = vec![0.; f.len()];
    let process = |(out, freq): (&mut [f64], &[f64])| -> Result<()> {
        check_cancel(cancel)?;
        for (value, &frequency) in out.iter_mut().zip(freq) {
            *value = evaluate(frequency);
        }
        Ok(())
    };
    // Independent frequency bins, bounded by the application's shared Rayon pool.
    if f.len() * (b.len() + a.len()) >= 1_000_000 {
        output
            .par_chunks_mut(256)
            .zip(f.par_chunks(256))
            .try_for_each(process)?;
    } else {
        for pair in output.chunks_mut(256).zip(f.chunks(256)) {
            process(pair)?;
        }
    }
    Ok(output)
}

pub fn sos_group_delay(sos: &Matrix, f: &[f64], cancel: &AtomicBool) -> Result<Vec<f64>> {
    let sections = sos_sections(sos)?;
    count(f.len(), sections.len() * 12, cancel)?;
    let mut output = vec![0.; f.len()];
    for section in sections {
        let delay = group_delay(&section.b, &section.a, f, cancel)?;
        for (out, value) in output.iter_mut().zip(delay) {
            *out += value;
        }
    }
    Ok(output)
}

/// Magnitude is linear; phase is radians; group delay is samples.
/// Time-domain responses start from zero state and require an explicit length.
pub fn analyze(
    kind: Analysis,
    b: &[f64],
    a: &[f64],
    n: usize,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    let mut filter = Filter::new(b, a)?;
    count(n, 2 * (b.len() + a.len()), cancel)?;
    if kind.time_domain() {
        let mut result = vec![if kind == Analysis::Step { 1. } else { 0. }; n];
        result[0] = 1.;
        filter.process(&mut result, cancel)?;
        if result.iter().any(|v| !v.is_finite()) {
            return Err("Filter response produced nonfinite values".into());
        }
        Ok(result)
    } else {
        let f = frequency_grid(n);
        if kind == Analysis::GroupDelay {
            return group_delay(b, a, &f, cancel);
        }
        let h = response(b, a, &f, cancel)?;
        Ok(if kind == Analysis::Phase {
            phase_response(&h)
        } else {
            h.iter().map(|v| v[0].hypot(v[1])).collect()
        })
    }
}

pub fn analyze_sos(
    kind: Analysis,
    sos: &Matrix,
    n: usize,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    let sections = sos_sections(sos)?;
    count(n, sections.len() * 12, cancel)?;
    if kind.time_domain() {
        let mut result = vec![if kind == Analysis::Step { 1. } else { 0. }; n];
        result[0] = 1.;
        for mut section in sections {
            section.process(&mut result, cancel)?;
        }
        if result.iter().any(|v| !v.is_finite()) {
            return Err("Filter response produced nonfinite values".into());
        }
        Ok(result)
    } else {
        let f = frequency_grid(n);
        if kind == Analysis::GroupDelay {
            return sos_group_delay(sos, &f, cancel);
        }
        let h = sos_response(sos, &f, cancel)?;
        Ok(if kind == Analysis::Phase {
            phase_response(&h)
        } else {
            h.iter().map(|v| v[0].hypot(v[1])).collect()
        })
    }
}
