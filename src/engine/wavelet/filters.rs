//! Wavelet name -> the four filters [LO_D, HI_D, LO_R, HI_R] of `wfilters`.
//! Orthogonal families store LO_R only: LO_D is it reversed, HI_R(k) =
//! (-1)^k LO_R(N-1-k) and HI_D is HI_R reversed. rbioX.Y reverses the
//! biorthogonal biorX.Y filters (LO_D = flip(LO_R of bior), ...).
use super::coeffs::{BIOR, ORTHO};
use crate::Result;

#[derive(Clone, Debug)]
pub(super) struct Bank {
    pub lo_d: Vec<f64>,
    pub hi_d: Vec<f64>,
    pub lo_r: Vec<f64>,
    pub hi_r: Vec<f64>,
}

fn reversed(v: &[f64]) -> Vec<f64> {
    v.iter().rev().copied().collect()
}

pub(super) fn bank(name: &str) -> Result<Bank> {
    let lower = name.trim().to_ascii_lowercase();
    let key = if lower == "haar" {
        "db1"
    } else {
        lower.as_str()
    };
    if let Some((_, lo_r)) = ORTHO.iter().find(|(n, _)| *n == key) {
        let n = lo_r.len();
        let hi_r: Vec<f64> = (0..n)
            .map(|k| {
                let v = lo_r[n - 1 - k];
                if k % 2 == 0 { v } else { -v }
            })
            .collect();
        return Ok(Bank {
            lo_d: reversed(lo_r),
            hi_d: reversed(&hi_r),
            lo_r: lo_r.to_vec(),
            hi_r,
        });
    }
    let (family, order) = if let Some(o) = key.strip_prefix("bior") {
        (false, o)
    } else if let Some(o) = key.strip_prefix("rbio") {
        (true, o)
    } else {
        return Err(unknown(name));
    };
    let Some((_, f)) = BIOR.iter().find(|(n, _)| *n == order) else {
        return Err(unknown(name));
    };
    Ok(if family {
        Bank {
            lo_d: reversed(f[2]),
            hi_d: reversed(f[3]),
            lo_r: reversed(f[0]),
            hi_r: reversed(f[1]),
        }
    } else {
        Bank {
            lo_d: f[0].to_vec(),
            hi_d: f[1].to_vec(),
            lo_r: f[2].to_vec(),
            hi_r: f[3].to_vec(),
        }
    })
}

fn unknown(name: &str) -> String {
    format!(
        "Invalid wavelet name '{name}': use haar, db1..db20, sym2..sym20, coif1..coif5, \
         biorNr.Nd or rbioNr.Nd"
    )
}
