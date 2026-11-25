//! Reuse the engine's column-pivoted QR; never solve via normal equations.
use super::*;
use nalgebra::{DMatrix, DVector};
pub(super) struct Fit {
    pub b: Vec<f64>,
    pub se: Vec<f64>,
    pub cov: DMatrix<f64>,
    pub res: Vec<f64>,
    pub fitted: Vec<f64>,
    pub sse: f64,
    pub sst: f64,
    pub mse: f64,
    pub mean: f64,
    pub n: usize,
    pub k: usize,
    pub intercept: bool,
}
pub(super) fn fit(
    e: &mut Engine,
    n: &str,
    x: &DMatrix<f64>,
    y: &[f64],
    intercept: bool,
) -> Result<Fit> {
    let (rows, k) = x.shape();
    if rows <= k || k == 0 {
        return Err(err(
            n,
            "TooFewObservations",
            "Too few observations for the requested regression.",
        ));
    }
    if rows.saturating_mul(k) > MAX_DESIGN {
        return Err(err(
            n,
            "WorkspaceLimit",
            "Regression design exceeds 2000000 elements.",
        ));
    }
    work(e, n, rows.saturating_mul(k).saturating_mul(k))?;
    let ls = super::super::stats_models::regression::least_squares(
        e,
        x,
        &DVector::from_column_slice(y),
    )?;
    if ls.rank != k {
        return Err(err(
            n,
            "RankDeficient",
            "Regression design is rank deficient.",
        ));
    }
    let fitted = (x * &ls.b).as_slice().to_vec();
    let res: Vec<_> = y.iter().zip(&fitted).map(|(y, f)| y - f).collect();
    let sse = res.iter().map(|r| r * r).sum::<f64>();
    let mean = y.iter().sum::<f64>() / rows as f64;
    let sst = y
        .iter()
        .map(|y| (y - if intercept { mean } else { 0. }).powi(2))
        .sum::<f64>();
    let mse = sse / (rows - k) as f64;
    if !mse.is_finite() || sst <= 0. || mse <= f64::MIN_POSITIVE {
        return Err(err(
            n,
            "DegenerateData",
            "Regression requires nonconstant data and positive finite residual variance.",
        ));
    }
    let cov = &ls.covariance_factor * ls.covariance_factor.transpose() * mse;
    let se = (0..k).map(|i| cov[(i, i)].sqrt()).collect();
    Ok(Fit {
        b: ls.b.as_slice().to_vec(),
        se,
        cov,
        res,
        fitted,
        sse,
        sst,
        mse,
        mean,
        n: rows,
        k,
        intercept,
    })
}
/// Bartlett / Newey-West HAC estimate, with the denominator T at every lag.
pub(super) fn hac(e: &mut Engine, n: &str, res: &[f64], lags: usize) -> Result<(f64, Vec<f64>)> {
    work(e, n, res.len().saturating_mul(lags + 1))?;
    let mut ac = Vec::with_capacity(lags + 1);
    for lag in 0..=lags {
        e.check_cancel()?;
        ac.push(res[lag..].iter().zip(res).map(|(a, b)| a * b).sum::<f64>() / res.len() as f64);
    }
    let mut variance = ac[0];
    for (lag, &v) in ac.iter().enumerate().skip(1) {
        variance += 2. * (1. - lag as f64 / (lags + 1) as f64) * v;
    }
    if !variance.is_finite() || variance <= 0. {
        return Err(err(
            n,
            "DegenerateData",
            "Long-run variance must be positive and finite.",
        ));
    }
    Ok((variance, ac))
}

/// Newey-West coefficient standard errors for the KPSS deterministic regression.
/// The bread comes from the existing QR covariance factor, not a normal-equation
/// fit. Bartlett weights; no degrees-of-freedom correction to the HAC meat.
pub(super) fn hac_se(
    e: &mut Engine,
    n: &str,
    x: &DMatrix<f64>,
    f: &Fit,
    lags: usize,
) -> Result<Vec<f64>> {
    work(
        e,
        n,
        f.n.saturating_mul(lags + 1)
            .saturating_mul(f.k)
            .saturating_mul(f.k),
    )?;
    let mut meat = DMatrix::<f64>::zeros(f.k, f.k);
    for lag in 0..=lags {
        e.check_cancel()?;
        let weight = if lag == 0 {
            1.
        } else {
            1. - lag as f64 / (lags + 1) as f64
        };
        for t in lag..f.n {
            let cross = f.res[t] * f.res[t - lag] * weight;
            for j in 0..f.k {
                for k in 0..f.k {
                    meat[(j, k)] += cross
                        * (x[(t, j)] * x[(t - lag, k)]
                            + if lag == 0 {
                                0.
                            } else {
                                x[(t - lag, j)] * x[(t, k)]
                            });
                }
            }
        }
    }
    let bread = &f.cov / f.mse;
    let cov = &bread * meat * bread.transpose();
    Ok((0..f.k).map(|j| cov[(j, j)].max(0.).sqrt()).collect())
}
