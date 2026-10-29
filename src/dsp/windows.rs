//! Real double-precision windows for FIR design and spectral analysis.
//! Allocation is bounded; output owns one buffer and uses the shared Rayon pool.
mod cosine;

use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    Hann,
    Hamming,
    Blackman,
    BlackmanHarris,
    Nuttall,
    FlatTop,
    Bartlett,
    Triangular,
    Rectangular,
}

impl WindowKind {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "hann" => Self::Hann,
            "hamming" => Self::Hamming,
            "blackman" => Self::Blackman,
            "blackmanharris" => Self::BlackmanHarris,
            "nuttallwin" => Self::Nuttall,
            "flattopwin" => Self::FlatTop,
            "bartlett" => Self::Bartlett,
            "triang" => Self::Triangular,
            "rectwin" => Self::Rectangular,
            _ => return None,
        })
    }
    pub fn supports_periodic(self) -> bool {
        matches!(
            self,
            Self::Hann
                | Self::Hamming
                | Self::Blackman
                | Self::BlackmanHarris
                | Self::Nuttall
                | Self::FlatTop
        )
    }
}

/// MATLAB window lengths round nonnegative finite scalars to the nearest integer.
/// Reject excessive lengths before allocating, including an out-of-range rounded result.
pub fn window_length(value: f64) -> Result<usize> {
    if !value.is_finite() || value < 0. || value.round() > crate::matrix::MAX_ELEMENTS as f64 {
        return Err(
            "Window length must be finite, nonnegative and within the matrix element limit".into(),
        );
    }
    Ok(value.round() as usize)
}

/// Generate a column vector, including 0-by-1 and singleton windows.
/// Only generalized cosine windows accept periodic sampling. Inputs and outputs are f64.
pub fn window(kind: WindowKind, n: usize, periodic: bool, cancel: &AtomicBool) -> Result<Matrix> {
    super::check_cancel(cancel)?;
    if n > crate::matrix::MAX_ELEMENTS {
        return Err("Window exceeds matrix element limit".into());
    }
    if periodic && !kind.supports_periodic() {
        return Err("Periodic sampling is supported only for generalized cosine windows".into());
    }
    if n <= 1 {
        return Matrix::from_column_vec(n, 1, vec![1.; n]);
    }
    if matches!(
        kind,
        WindowKind::BlackmanHarris | WindowKind::Nuttall | WindowKind::FlatTop
    ) {
        return cosine::generate(kind, n, periodic, cancel);
    }
    let mut output = vec![0.; n];
    let denominator = if periodic { n } else { n - 1 };
    let sample = |index: usize| -> f64 {
        match kind {
            WindowKind::BlackmanHarris | WindowKind::Nuttall | WindowKind::FlatTop => {
                unreachable!()
            }
            WindowKind::Rectangular => 1.,
            WindowKind::Bartlett => 2. * index.min(n - 1 - index) as f64 / (n - 1) as f64,
            WindowKind::Triangular => {
                let index = index.min(n - 1 - index);
                if n.is_multiple_of(2) {
                    (2 * index + 1) as f64 / n as f64
                } else {
                    (2 * index + 2) as f64 / (n + 1) as f64
                }
            }
            WindowKind::Hann | WindowKind::Hamming | WindowKind::Blackman => {
                // Folding enforces exact symmetry and keeps the sine argument in [0, pi/2].
                // The sine-squared forms avoid cancellation at the zero endpoints.
                let folded = index.min(denominator - index);
                let sine = (std::f64::consts::PI * folded as f64 / denominator as f64).sin();
                let squared = sine * sine;
                match kind {
                    WindowKind::Hann => squared,
                    WindowKind::Hamming => 0.08 + 0.92 * squared,
                    WindowKind::Blackman => squared * (0.36 + 0.64 * squared),
                    _ => unreachable!(),
                }
            }
        }
    };
    let fill = |(chunk_index, chunk): (usize, &mut [f64])| -> Result<()> {
        super::check_cancel(cancel)?;
        for (offset, value) in chunk.iter_mut().enumerate() {
            *value = sample(chunk_index * 4096 + offset);
        }
        Ok(())
    };
    // Release one/eight-worker measurements support the 65,536-element threshold.
    // See research/performance/window-functions for sizes, samples and limitations.
    // Cheap linear windows stay serial to avoid task-scheduling overhead.
    if kind.supports_periodic() && n >= 65_536 && rayon::current_num_threads() > 1 {
        output.par_chunks_mut(4096).enumerate().try_for_each(fill)?;
    } else {
        for chunk in output.chunks_mut(4096).enumerate() {
            fill(chunk)?;
        }
    }
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(n, 1, output)
}
