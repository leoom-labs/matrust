//! Parameterised windows: gausswin, tukeywin, chebwin, taylorwin, barthannwin,
//! bohmanwin, parzenwin. Symmetric, real f64 column vectors. Formulas follow the
//! public function descriptions; chebwin uses the Dolph-Chebyshev frequency
//! sampling + DFT construction normalised to a peak of 1.
use crate::{Matrix, Result};
use rustfft::{FftPlanner, num_complex::Complex};
use std::f64::consts::PI;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamWindow {
    Gauss(f64),
    Tukey(f64),
    Cheb(f64),
    Taylor(usize, f64),
    BartHann,
    Bohman,
    Parzen,
}

pub fn param_window(kind: ParamWindow, n: usize, cancel: &AtomicBool) -> Result<Matrix> {
    super::check_cancel(cancel)?;
    if n > crate::matrix::MAX_ELEMENTS {
        return Err("Window exceeds matrix element limit".into());
    }
    if n <= 1 {
        return Matrix::from_column_vec(n, 1, vec![1.; n]);
    }
    let m = (n - 1) as f64;
    let w: Vec<f64> = match kind {
        ParamWindow::Gauss(alpha) => (0..n)
            .map(|k| {
                let t = alpha * (k as f64 - m / 2.) / (m / 2.);
                (-0.5 * t * t).exp()
            })
            .collect(),
        ParamWindow::Tukey(r) => {
            if r <= 0. {
                vec![1.; n]
            } else {
                let r = r.min(1.);
                (0..n)
                    .map(|k| {
                        // Fold to the left half for exact symmetry.
                        let x = k.min(n - 1 - k) as f64 / m;
                        if x < r / 2. {
                            0.5 * (1. + (2. * PI / r * (x - r / 2.)).cos())
                        } else {
                            1.
                        }
                    })
                    .collect()
            }
        }
        ParamWindow::BartHann => (0..n)
            .map(|k| {
                let x = k.min(n - 1 - k) as f64 / m - 0.5;
                0.62 - 0.48 * x.abs() + 0.38 * (2. * PI * x).cos()
            })
            .collect(),
        ParamWindow::Bohman => (0..n)
            .map(|k| {
                let x = (2. * k.min(n - 1 - k) as f64 / m - 1.).abs();
                ((1. - x) * (PI * x).cos() + (PI * x).sin() / PI).max(0.)
            })
            .collect(),
        ParamWindow::Parzen => {
            let half = n as f64 / 2.;
            (0..n)
                .map(|k| {
                    let a = (k as f64 - m / 2.).abs();
                    let u = a / half;
                    if a <= m / 4. {
                        1. - 6. * u * u + 6. * u * u * u
                    } else {
                        2. * (1. - u).powi(3)
                    }
                })
                .collect()
        }
        ParamWindow::Taylor(nbar, sll) => taylor(n, nbar, sll, cancel)?,
        ParamWindow::Cheb(r) => cheb(n, r),
    };
    super::check_cancel(cancel)?;
    Matrix::from_column_vec(n, 1, w)
}

/// Upper bound on taylorwin's multiply-adds: (NBAR-1)^2 for the coefficients
/// plus (NBAR-1) per distinct (folded) sample. NBAR = 1e4 with L = 1e5 needs
/// ~6e8; the bound only rejects NBAR far beyond any useful sidelobe count.
pub const TAYLOR_MAX_WORK: f64 = 1e9;

fn taylor(n: usize, nbar: usize, sll: f64, cancel: &AtomicBool) -> Result<Vec<f64>> {
    let terms = nbar.saturating_sub(1);
    let half = n.div_ceil(2);
    let work = terms as f64 * (terms as f64 + half as f64);
    if work > TAYLOR_MAX_WORK {
        return Err(format!(
            "taylorwin: NBAR too large (work (NBAR-1)*(NBAR-1+ceil(L/2)) = {work:.3e} exceeds {TAYLOR_MAX_WORK:.0e})"
        ));
    }
    // Poll cancellation about every 2^20 multiply-adds.
    let stride = ((1usize << 20) / terms.max(1)).max(1);
    let a = (10f64.powf(-sll / 20.)).acosh() / PI;
    let nb = nbar as f64;
    let sp2 = nb * nb / (a * a + (nb - 0.5) * (nb - 0.5));
    let mut f = Vec::with_capacity(terms);
    for mi in 1..nbar {
        if mi % stride == 0 {
            super::check_cancel(cancel)?;
        }
        let mf = mi as f64;
        let mut num = 1.;
        let mut den = 1.;
        for ni in 1..nbar {
            let nf = ni as f64;
            num *= 1. - mf * mf / sp2 / (a * a + (nf - 0.5) * (nf - 0.5));
            if ni != mi {
                den *= 1. - mf * mf / (nf * nf);
            }
        }
        let sign = if mi % 2 == 1 { 1. } else { -1. };
        f.push(sign * num / (2. * den));
    }
    let len = n as f64;
    // Folded samples k and n-1-k are equal: evaluate the first half, mirror.
    let mut w = Vec::with_capacity(n);
    for k in 0..half {
        if k % stride == 0 {
            super::check_cancel(cancel)?;
        }
        let xi = (k as f64 - len / 2. + 0.5) / len;
        w.push(
            1. + 2.
                * f.iter()
                    .enumerate()
                    .map(|(i, fm)| fm * (2. * PI * (i + 1) as f64 * xi).cos())
                    .sum::<f64>(),
        );
    }
    for k in half..n {
        w.push(w[n - 1 - k]);
    }
    Ok(w)
}

fn cheb(n: usize, r: f64) -> Vec<f64> {
    let order = (n - 1) as f64;
    let beta = ((10f64.powf(r.abs() / 20.)).acosh() / order).cosh();
    let mut p: Vec<Complex<f64>> = (0..n)
        .map(|k| {
            let x = beta * (PI * k as f64 / n as f64).cos();
            let v = if x > 1. {
                (order * x.acosh()).cosh()
            } else if x < -1. {
                let sign = if n % 2 == 1 { 1. } else { -1. };
                sign * (order * (-x).acosh()).cosh()
            } else {
                (order * x.acos()).cos()
            };
            if n % 2 == 1 {
                Complex::new(v, 0.)
            } else {
                Complex::from_polar(v, PI / n as f64 * k as f64)
            }
        })
        .collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut p);
    let re: Vec<f64> = p.iter().map(|c| c.re).collect();
    let mut w = Vec::with_capacity(n);
    if n % 2 == 1 {
        let h = n.div_ceil(2);
        w.extend(re[1..h].iter().rev());
        w.extend(&re[..h]);
    } else {
        let h = n / 2 + 1;
        w.extend(re[1..h].iter().rev());
        w.extend(&re[1..h]);
    }
    // Exact symmetry.
    for k in 0..n / 2 {
        let avg = 0.5 * (w[k] + w[n - 1 - k]);
        w[k] = avg;
        w[n - 1 - k] = avg;
    }
    let peak = w.iter().cloned().fold(f64::MIN, f64::max);
    w.iter().map(|v| v / peak).collect()
}
