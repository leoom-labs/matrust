//! Stable symmetric Kaiser windows, real f64 only; no new dependencies.
use crate::{Matrix, Result};
use rayon::prelude::*;
use std::sync::atomic::AtomicBool;

// Independently derived from DLMF 10.25.2 (nu=0). For x<50 all terms are
// positive, and 128 terms comfortably exceed f64 convergence requirements.
fn i0_small(x: f64) -> f64 {
    let quarter_square = (0.5 * x) * (0.5 * x);
    let mut term = 1.;
    let mut sum = 1.;
    let mut correction = 0.;
    for k in 1..=128 {
        term *= quarter_square / ((k * k) as f64);
        let adjusted = term - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
        if term <= sum * (f64::EPSILON * 0.125) {
            break;
        }
    }
    sum
}
// Scaled I0 asymptotic correction: I0(x) ~ exp(x)/sqrt(2*pi*x) * sum.
// DLMF 10.40.1 gives this recurrence at nu=0. x>=50; terms stop well
// before the divergent tail, with a hard iteration bound for cancellation.
fn i0_asymptotic(x: f64) -> f64 {
    let inverse = 1. / x;
    let mut term = 1.;
    let mut sum = 1.;
    let mut correction = 0.;
    for k in 1..=64 {
        let odd = (2 * k - 1) as f64;
        term *= (odd * odd / (8. * k as f64)) * inverse;
        let adjusted = term - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
        if term <= sum * (f64::EPSILON * 0.125) {
            break;
        }
    }
    sum
}
struct KaiserParameters {
    beta: f64,
    denominator: f64,
    small_prefactor: f64,
}
impl KaiserParameters {
    fn new(beta: f64) -> Self {
        let beta = beta.abs();
        let denominator = if beta < 50. {
            i0_small(beta)
        } else {
            i0_asymptotic(beta)
        };
        Self {
            beta,
            denominator,
            small_prefactor: if beta < 50. {
                0.
            } else {
                libm::sqrt(beta) * libm::sqrt(std::f64::consts::TAU) / denominator
            },
        }
    }

    fn sample(&self, n: usize, index: usize) -> f64 {
        if n <= 1 || self.beta == 0. {
            return 1.;
        }
        let d = n - 1;
        let folded = index.min(d - index);
        let distance = d - 2 * folded;
        if distance == 0 {
            return 1.;
        }
        // With n<=MAX_ELEMENTS=2^24, this integer radicand fits exactly in f64.
        // It avoids loss near the endpoints from forming 1-u*u.
        let root = libm::sqrt((4u64 * folded as u64 * (d - folded) as u64) as f64) / d as f64;
        let argument = self.beta * root;
        let result = if self.beta < 50. {
            i0_small(argument) / self.denominator
        } else if argument < 50. {
            // Splitting the exponent and multiplying the prefactor between
            // halves retains subnormals lost by exp(-beta) alone. The I0
            // power series is bounded by I0(50), so this prefactor is finite.
            let exponential = libm::exp(-0.5 * self.beta);
            (exponential * (i0_small(argument) * self.small_prefactor)) * exponential
        } else {
            let u = distance as f64 / d as f64;
            let difference = -self.beta * (u * u / (1. + root));
            // Stable beta*(root-1), including roots rounded to exactly one.
            let prefactor = i0_asymptotic(argument) / self.denominator / libm::sqrt(root);
            if difference > -700. {
                libm::exp(difference) * prefactor
            } else {
                let exponential = libm::exp(0.5 * difference);
                (exponential * prefactor) * exponential
            }
        };
        // The mathematical window lies in [0,1]; suppress only endpoint roundoff.
        result.min(1.)
    }
}

/// Symmetric n-by-1 Kaiser window. beta defaults to 0.5 in the script dispatcher.
/// Finite negative beta has the same result as positive beta (I0 is even).
/// All finite beta magnitudes are supported; nonfinite beta is rejected even for n=0/1.
pub fn kaiser(n: usize, beta: f64, cancel: &AtomicBool) -> Result<Matrix> {
    if n > crate::matrix::MAX_ELEMENTS {
        return Err("Kaiser window exceeds matrix element limit".into());
    }
    if !beta.is_finite() {
        return Err("Kaiser beta must be a finite real scalar".into());
    }
    super::check_cancel(cancel)?;
    if n <= 1 {
        return Matrix::from_column_vec(n, 1, vec![1.; n]);
    }
    if beta == 0. {
        // Initialize rectangular output once, retaining bounded cancellation work.
        let mut output = Vec::with_capacity(n);
        for start in (0..n).step_by(4096) {
            super::check_cancel(cancel)?;
            output.resize((start + 4096).min(n), 1.);
        }
        super::check_cancel(cancel)?;
        return Matrix::from_column_vec(n, 1, output);
    }
    let prepared = KaiserParameters::new(beta);
    let mut output = vec![0.; n];
    let (left, tail) = output.split_at_mut(n / 2);
    let odd = n % 2;
    if odd != 0 {
        tail[0] = 1.;
    }
    let right = &mut tail[odd..];
    let fill = |(index, (front, back)): (usize, (&mut [f64], &mut [f64]))| -> Result<()> {
        super::check_cancel(cancel)?;
        for (offset, (a, b)) in front.iter_mut().zip(back.iter_mut().rev()).enumerate() {
            let value = prepared.sample(n, index * 4096 + offset);
            *a = value;
            *b = value;
        }
        Ok(())
    };
    // Candidate only: release one/eight-worker measurements must select this threshold.
    if n >= 65_536 && rayon::current_num_threads() > 1 {
        left.par_chunks_mut(4096)
            .zip(right.par_rchunks_mut(4096))
            .enumerate()
            .try_for_each(fill)?;
    } else {
        for pair in left
            .chunks_mut(4096)
            .zip(right.rchunks_mut(4096))
            .enumerate()
        {
            fill(pair)?;
        }
    }
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(n, 1, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_high_precision_samples_include_max_length_without_large_allocation() {
        let mut count = 0;
        for line in include_str!("../../tests/kaiser_selected_oracle.csv")
            .lines()
            .skip(1)
        {
            let f: Vec<_> = line.split(',').collect();
            let n = f[0].parse().unwrap();
            let index = f[1].parse().unwrap();
            let beta = f[2].parse().unwrap();
            let expected: f64 = f[3].parse().unwrap();
            let actual = KaiserParameters::new(beta).sample(n, index);
            assert!(actual.is_finite() && (0. ..=1.).contains(&actual));
            assert!(
                (actual - expected).abs() <= 3e-13 * expected.abs() + f64::from_bits(8),
                "row{count}: {actual} != {expected} n={n} index={index} beta={beta}"
            );
            count += 1;
        }
        assert_eq!(count, 672);
    }
}
