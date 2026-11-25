//! Variational mode decomposition (Dragomiretskiy & Zosso 2014): ADMM in the
//! frequency domain on the signal mirrored by half its length at each end
//! (2N samples). On the one-sided spectrum f = k/(2N), k = 0..N-1:
//!   U_k <- (X - sum_{i<k} U_i(new) - sum_{i>k} U_i(old) + L/2) / (1 + 2a(2 pi (f - w_k))^2)
//!   w_k <- sum f |U_k|^2 / sum |U_k|^2,   L <- L + tau (X - sum U_k).
//! Stops when the mean squared change per sample of the modes is below
//! AbsoluteTolerance and the mean relative change ||dU_k||^2/||U_k||^2 below
//! RelativeTolerance, or after MaxIterations. Modes come out sorted by
//! ascending central frequency; the residual is x minus their sum.
use super::super::poly_interp::real_matrix;
use super::super::{Engine, Value};
use super::mra::{
    mra_bool, mra_column, mra_columns, mra_count, mra_fft, mra_options, mra_scalar, mra_signal,
    mra_struct, mra_text,
};
use crate::Result;
use rustfft::num_complex::Complex;

#[derive(Clone, Copy, PartialEq)]
enum Init {
    Peaks,
    Random,
    Grid,
}

struct Opts {
    abs_tol: f64,
    rel_tol: Option<f64>,
    max_iter: usize,
    k: usize,
    alpha: f64,
    tau: f64,
    init: Init,
    centers: Option<Vec<f64>>,
}

pub(super) fn vmd_builtin(e: &mut Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    const WHO: &str = "vmd";
    let Some(first) = args.first() else {
        return Err("vmd: not enough input arguments".into());
    };
    let x = mra_signal(first, WHO, 2)?;
    let mut o = Opts {
        abs_tol: 5e-6,
        rel_tol: None,
        max_iter: 500,
        k: 5,
        alpha: 1000.,
        tau: 0.01,
        init: Init::Peaks,
        centers: None,
    };
    const NAMES: [&str; 11] = [
        "AbsoluteTolerance",
        "RelativeTolerance",
        "MaxIterations",
        "NumIMFs",
        "PenaltyFactor",
        "LMUpdateRate",
        "InitializeMethod",
        "CentralFrequencies",
        "InitialIMFs",
        "InitialLM",
        "Display",
    ];
    let positive = |v: &Value, what: &str| -> Result<f64> {
        let t = mra_scalar(v, WHO, what)?;
        if t <= 0. {
            return Err(format!("vmd: {what} must be positive"));
        }
        Ok(t)
    };
    for (i, v) in mra_options(&args[1..], &NAMES, WHO)? {
        match i {
            0 => o.abs_tol = positive(v, "AbsoluteTolerance")?,
            1 => o.rel_tol = Some(positive(v, "RelativeTolerance")?),
            2 => o.max_iter = mra_count(v, WHO, "MaxIterations", 1_000_000)?,
            3 => o.k = mra_count(v, WHO, "NumIMFs", 1000)?,
            4 => o.alpha = positive(v, "PenaltyFactor")?,
            5 => {
                o.tau = mra_scalar(v, WHO, "LMUpdateRate")?;
                if o.tau < 0. {
                    return Err("vmd: LMUpdateRate must be nonnegative".into());
                }
            }
            6 => {
                let t = mra_text(v)
                    .ok_or("vmd: InitializeMethod must be 'peaks', 'random' or 'grid'")?;
                o.init = match t.to_ascii_lowercase().as_str() {
                    "peaks" => Init::Peaks,
                    "random" => Init::Random,
                    "grid" => Init::Grid,
                    _ => {
                        return Err(format!(
                            "vmd: InitializeMethod must be 'peaks', 'random' or 'grid', not '{t}'"
                        ));
                    }
                };
            }
            7 => {
                let m = real_matrix(v, WHO)?;
                let c = m.data().to_vec();
                if c.iter().any(|f| !(0. ..=0.5).contains(f)) {
                    return Err("vmd: CentralFrequencies must lie in [0, 0.5]".into());
                }
                o.centers = Some(c);
            }
            8 => return Err("vmd: InitialIMFs is not supported yet".into()),
            9 => return Err("vmd: InitialLM is not supported yet".into()),
            _ => {
                if mra_bool(v, WHO, "Display")? {
                    return Err("vmd: Display=true is not supported yet".into());
                }
            }
        }
    }
    if let Some(c) = &o.centers
        && c.len() != o.k
    {
        return Err(format!(
            "vmd: CentralFrequencies must have NumIMFs = {} elements",
            o.k
        ));
    }
    let n = x.len();
    let r = vmd(e, &x, &o)?;
    let residual: Vec<f64> = (0..n)
        .map(|t| x[t] - r.modes.iter().map(|m| m[t]).sum::<f64>())
        .collect();
    let mut outs = vec![mra_columns(n, &r.modes)?, mra_column(residual)?];
    if nargout > 2 {
        let lm_data: Vec<crate::Complex64> =
            r.lm.iter()
                .map(|c| crate::Complex64::new(c.re, c.im))
                .collect();
        let len = lm_data.len();
        let lm = Value::Complex(crate::ComplexMatrix::from_column_vec(
            len, 1, lm_data, &e.cancel,
        )?);
        outs.push(mra_struct(
            e,
            vec![
                (
                    "ExitFlag",
                    Value::Matrix(crate::Matrix::scalar(r.exit_flag)),
                ),
                ("CentralFrequencies", mra_column(r.centers)?),
                (
                    "NumIterations",
                    Value::Matrix(crate::Matrix::scalar(r.iterations as f64)),
                ),
                (
                    "AbsoluteImprovement",
                    Value::Matrix(crate::Matrix::scalar(r.abs_imp)),
                ),
                (
                    "RelativeImprovement",
                    Value::Matrix(crate::Matrix::scalar(r.rel_imp)),
                ),
                ("LagrangeMultiplier", lm),
            ],
        )?);
    }
    Ok(outs)
}

struct VmdResult {
    modes: Vec<Vec<f64>>,
    centers: Vec<f64>,
    exit_flag: f64,
    iterations: usize,
    abs_imp: f64,
    rel_imp: f64,
    lm: Vec<Complex<f64>>,
}

fn vmd(e: &mut Engine, x: &[f64], o: &Opts) -> Result<VmdResult> {
    let n = x.len();
    let h1 = n / 2;
    let t = n.checked_mul(2).ok_or("Matrix dimensions overflow")?;
    // Mirrored spectrum (2N complex) and the NumIMFs-by-N complex state,
    // before either buffer is allocated. The real IMF matrix is the same shape.
    super::mra::mra_admit_complex(e, "vmd", t, 1)?;
    super::mra::mra_admit_complex(e, "vmd", o.k, n)?;
    super::mra::mra_admit(e, "vmd", n, o.k)?;
    let mut buf: Vec<Complex<f64>> = Vec::with_capacity(t);
    buf.extend(x[..h1].iter().rev().map(|&v| Complex::new(v, 0.)));
    buf.extend(x.iter().map(|&v| Complex::new(v, 0.)));
    buf.extend(x[h1..].iter().rev().map(|&v| Complex::new(v, 0.)));
    mra_fft(&mut buf, false);
    let xs: Vec<Complex<f64>> = buf[..n].to_vec();
    let freqs: Vec<f64> = (0..n).map(|k| k as f64 / t as f64).collect();
    let k = o.k;
    let mut omega = match &o.centers {
        Some(c) => c.clone(),
        None => initial_centers(e, &xs, &freqs, k, o.init),
    };
    let rel_tol = o.rel_tol.unwrap_or(o.abs_tol * 1e3);
    let mut u = vec![vec![Complex::new(0., 0.); n]; k];
    let mut lam = vec![Complex::new(0., 0.); n];
    let mut sum: Vec<Complex<f64>> = vec![Complex::new(0., 0.); n];
    let mut iterations = 0;
    let (mut abs_imp, mut rel_imp) = (f64::INFINITY, f64::INFINITY);
    let mut converged = false;
    let two_pi = 2. * std::f64::consts::PI;
    while iterations < o.max_iter {
        e.check_cancel()?;
        iterations += 1;
        let (mut a_acc, mut r_acc) = (0., 0.);
        for m in 0..k {
            let mut diff = 0.;
            let mut old = 0.;
            let w = omega[m];
            let mut power = 0.;
            let mut moment = 0.;
            for j in 0..n {
                let prev = u[m][j];
                let others = sum[j] - prev;
                let d = two_pi * (freqs[j] - w);
                let new = (xs[j] - others + lam[j] * 0.5) / (1. + 2. * o.alpha * d * d);
                u[m][j] = new;
                sum[j] = others + new;
                diff += (new - prev).norm_sqr();
                old += prev.norm_sqr();
                let p = new.norm_sqr();
                power += p;
                moment += p * freqs[j];
            }
            if power > 0. {
                omega[m] = moment / power;
            }
            // Full-spectrum energy is twice the one-sided one; Parseval / T^2 per sample.
            a_acc += 2. * diff / (t as f64 * t as f64);
            r_acc += if old > 0. {
                diff / old
            } else if diff > 0. {
                f64::INFINITY
            } else {
                0.
            };
        }
        for j in 0..n {
            lam[j] += (xs[j] - sum[j]) * o.tau;
        }
        abs_imp = a_acc / k as f64;
        rel_imp = r_acc / k as f64;
        if abs_imp < o.abs_tol && rel_imp < rel_tol {
            converged = true;
            break;
        }
    }
    // Back to time: Hermitian full spectrum, keep the middle N samples.
    let mut modes: Vec<(f64, Vec<f64>)> = Vec::with_capacity(k);
    let inv_t = 1. / t as f64;
    for m in 0..k {
        let mut full = vec![Complex::new(0., 0.); t];
        full[..n].copy_from_slice(&u[m]);
        for j in 1..n {
            full[t - j] = u[m][j].conj();
        }
        mra_fft(&mut full, true);
        let mode: Vec<f64> = full[h1..h1 + n].iter().map(|c| c.re * inv_t).collect();
        modes.push((omega[m], mode));
    }
    modes.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(VmdResult {
        centers: modes.iter().map(|m| m.0).collect(),
        modes: modes.into_iter().map(|m| m.1).collect(),
        exit_flag: if converged { 1. } else { 0. },
        iterations,
        abs_imp,
        rel_imp,
        lm: lam,
    })
}

/// Initial central frequencies (cycles/sample), ascending.
fn initial_centers(
    e: &mut Engine,
    xs: &[Complex<f64>],
    freqs: &[f64],
    k: usize,
    init: Init,
) -> Vec<f64> {
    let mut w: Vec<f64> = match init {
        Init::Grid => (0..k).map(|i| 0.5 * i as f64 / k as f64).collect(),
        Init::Random => (0..k).map(|_| 0.5 * e.random.next_f64()).collect(),
        Init::Peaks => {
            let mag: Vec<f64> = xs.iter().map(|c| c.norm()).collect();
            let mut peaks: Vec<usize> = (1..mag.len().saturating_sub(1))
                .filter(|&j| mag[j] > mag[j - 1] && mag[j] >= mag[j + 1])
                .collect();
            peaks.sort_by(|&a, &b| mag[b].total_cmp(&mag[a]).then(a.cmp(&b)));
            let mut w: Vec<f64> = peaks.iter().take(k).map(|&j| freqs[j]).collect();
            while w.len() < k {
                w.push(0.5 * e.random.next_f64());
            }
            w
        }
    };
    w.sort_by(f64::total_cmp);
    w
}
