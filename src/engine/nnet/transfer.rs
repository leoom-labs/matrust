//! Stateless transfer functions, with column-wise softmax and full Jacobians.
use crate::{Matrix, Result};
use std::sync::atomic::AtomicBool;
pub const NAMES: &[&str] = &[
    "tansig", "logsig", "purelin", "softmax", "poslin", "satlin", "hardlim", "radbas",
];
pub fn apply(name: &str, n: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    crate::dsp::check_cancel(cancel)?;
    if !NAMES.contains(&name) {
        return Err(format!("Unsupported transferFcn '{name}'"));
    }
    let mut v = n.data().to_vec();
    if name == "softmax" {
        for col in v.chunks_mut(n.rows().max(1)) {
            crate::dsp::check_cancel(cancel)?;
            let m = col.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let mut sum = 0.;
            for x in col.iter_mut() {
                *x = (*x - m).exp();
                sum += *x;
            }
            for x in col {
                *x /= sum;
            }
        }
    } else {
        for (i, x) in v.iter_mut().enumerate() {
            if i.is_multiple_of(4096) {
                crate::dsp::check_cancel(cancel)?;
            }
            *x = match name {
                "tansig" => x.tanh(),
                "logsig" => {
                    if *x >= 0. {
                        1. / (1. + (-*x).exp())
                    } else {
                        let z = x.exp();
                        z / (1. + z)
                    }
                }
                "purelin" => *x,
                "poslin" => {
                    if x.is_nan() {
                        *x
                    } else {
                        x.max(0.)
                    }
                }
                "satlin" => x.clamp(0., 1.),
                "hardlim" => f64::from(*x >= 0.),
                "radbas" => (-*x * *x).exp(),
                _ => unreachable!(),
            };
        }
    }
    Matrix::from_column_vec(n.rows(), n.cols(), v)
}
pub fn derivative(name: &str, n: &Matrix, a: &Matrix, cancel: &AtomicBool) -> Result<Matrix> {
    if n.rows() != a.rows() || n.cols() != a.cols() {
        return Err("Transfer derivative: N and A sizes must match".into());
    }
    if name == "softmax" {
        let s = n.rows();
        let size = Matrix::checked_len(
            s,
            s.checked_mul(n.cols())
                .ok_or("Derivative dimensions overflow")?,
        )?;
        let mut v = vec![0.; size];
        for q in 0..n.cols() {
            crate::dsp::check_cancel(cancel)?;
            for j in 0..s {
                for i in 0..s {
                    v[q * s * s + j * s + i] =
                        a.data()[q * s + i] * (f64::from(i == j) - a.data()[q * s + j]);
                }
            }
        }
        return Matrix::from_column_vec(s, s * n.cols(), v)?.with_dims(&[s, s, n.cols()]);
    }
    let mut v = Vec::with_capacity(n.len());
    for (i, (&x, &y)) in n.data().iter().zip(a.data()).enumerate() {
        if i.is_multiple_of(4096) {
            crate::dsp::check_cancel(cancel)?;
        }
        v.push(match name {
            "tansig" => 1. - y * y,
            "logsig" => y * (1. - y),
            "purelin" => 1.,
            "poslin" => f64::from(x >= 0.),
            "satlin" => f64::from((0. ..=1.).contains(&x)),
            "hardlim" => 0.,
            "radbas" => {
                if y == 0. && x.is_finite() {
                    0.
                } else {
                    -2. * x * y
                }
            }
            _ => return Err(format!("Unsupported transferFcn '{name}'")),
        });
    }
    Matrix::from_column_vec(n.rows(), n.cols(), v)
}
