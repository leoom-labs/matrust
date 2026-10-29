// Frozen pre-specialization filtering implementation, 2026-10-15. Benchmark/reference only.
#![allow(dead_code)]
use matrust::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
const MAX_ORDER: usize = 4096;
const MAX_WORK: usize = 256_000_000;
#[derive(Clone, Debug)]
pub struct Filter {
    b: std::sync::Arc<[f64]>,
    a: std::sync::Arc<[f64]>,
    state: Vec<f64>,
    feedback: bool,
}
impl Filter {
    pub fn new(b: &[f64], a: &[f64]) -> Result<Self> {
        if b.is_empty()
            || a.is_empty()
            || b.len().max(a.len()) > MAX_ORDER + 1
            || a[0] == 0.
            || b.iter().chain(a).any(|v| !v.is_finite())
        {
            return Err("Filter coefficients must be finite nonempty vectors with nonzero a(1), order at most 4096".into());
        }
        let n = b.len().max(a.len());
        let mut numerator = vec![0.; n];
        let mut denominator = vec![0.; n];
        for (to, from) in numerator.iter_mut().zip(b) {
            *to = *from / a[0];
        }
        for (to, from) in denominator.iter_mut().zip(a) {
            *to = *from / a[0];
        }
        if numerator.iter().chain(&denominator).any(|v| !v.is_finite()) {
            return Err("Normalizing filter coefficients overflowed".into());
        }
        Ok(Self {
            feedback: denominator[1..].iter().any(|v| *v != 0.),
            b: numerator.into(),
            a: denominator.into(),
            state: vec![0.; n - 1],
        })
    }
    pub fn state(&self) -> &[f64] {
        &self.state
    }
    pub fn set_state(&mut self, state: &[f64]) -> Result<()> {
        if state.len() != self.state.len() || state.iter().any(|v| !v.is_finite()) {
            return Err("Invalid filter initial state".into());
        }
        self.state.copy_from_slice(state);
        Ok(())
    }
    pub fn reset(&mut self) {
        self.state.fill(0.);
    }
    #[inline]
    pub fn sample(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.state.first().copied().unwrap_or(0.);
        let n = self.state.len();
        if n > 0 {
            if self.feedback {
                for j in 0..n - 1 {
                    self.state[j] = self.state[j + 1] + self.b[j + 1] * x - self.a[j + 1] * y;
                }
                self.state[n - 1] = self.b[n] * x - self.a[n] * y;
            } else {
                // FIR delays contain contributions from inputs only. A padded
                // zero feedback term would incorrectly feed 0*NaN back forever.
                for j in 0..n - 1 {
                    self.state[j] = self.state[j + 1] + self.b[j + 1] * x;
                }
                self.state[n - 1] = self.b[n] * x;
            }
        }
        y
    }
    pub fn process(&mut self, data: &mut [f64], cancel: &AtomicBool) -> Result<()> {
        for chunk in data.chunks_mut(1024) {
            check_cancel(cancel)?;
            for x in chunk {
                *x = self.sample(*x);
            }
        }
        Ok(())
    }
}

pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}
pub fn vector(m: &Matrix) -> Result<&[f64]> {
    if m.is_empty() || (m.rows() > 1 && m.cols() > 1) {
        Err("Expected a nonempty coefficient vector".into())
    } else {
        Ok(m.data())
    }
}

fn channels<F>(x: &Matrix, cost: usize, cancel: &AtomicBool, process: F) -> Result<Matrix>
where
    F: Fn(&mut [f64]) -> Result<()> + Sync,
{
    if x.len().saturating_mul(cost) > MAX_WORK {
        return Err("Filter operation exceeds work limit; process shorter chunks".into());
    }
    check_cancel(cancel)?;
    if x.is_empty() {
        return Ok(x.clone());
    }
    let mut data = x.data().to_vec();
    let rows = if x.rows() == 1 { x.len() } else { x.rows() };
    // The threshold is benchmarked in benches/dsp.rs; only independent channels parallelize.
    if x.cols() > 1 && x.rows() > 1 && x.len().saturating_mul(cost) >= 1_000_000 {
        data.par_chunks_mut(rows).try_for_each(&process)?;
    } else {
        for channel in data.chunks_mut(rows) {
            process(channel)?;
        }
    }
    Matrix::from_column_vec(x.rows(), x.cols(), data)
}
pub fn sos_sections(sos: &Matrix) -> Result<Vec<Filter>> {
    if sos.cols() != 6 || sos.rows() == 0 || sos.rows() > 64 {
        return Err("SOS must have 1..64 rows and six columns [b0 b1 b2 a0 a1 a2]".into());
    }
    (0..sos.rows())
        .map(|r| {
            let v: Vec<_> = (0..6).map(|c| sos.data()[c * sos.rows() + r]).collect();
            Filter::new(&v[..3], &v[3..])
        })
        .collect()
}
pub fn sosfilt(sos: &Matrix, x: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    let sections = sos_sections(sos)?;
    channels(x, sections.len() * 3, cancel, |channel| {
        for mut filter in sections.clone() {
            filter.process(channel, cancel)?;
        }
        Ok(())
    })
}
