use super::super::stats_dist::families::Family;
use super::*;
use nalgebra::DMatrix;
pub(super) fn call(
    e: &mut Engine,
    n: &str,
    a: &[Value],
    _out: &mut Execution,
) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(err(n, "NotEnoughInputs", "Not enough input arguments."));
    }
    let y = vector(e, n, &a[0])?;
    let count = y.data().iter().filter(|v| !v.is_nan()).count();
    if count < 3 {
        return Err(err(
            n,
            "TooFewObservations",
            "At least three observations are required.",
        ));
    }
    if n == "archtest" && count != y.len() {
        return Err(err(
            n,
            "MissingData",
            "ARCH test does not support NaN observations.",
        ));
    }
    let allowed = if n == "lbqtest" {
        vec!["lags", "alpha", "dof"]
    } else {
        vec!["lags", "alpha"]
    };
    let mut o = Options::new(n, &a[1..], &allowed)?;
    let lags = o.nums(
        e,
        n,
        "lags",
        if n == "lbqtest" {
            20.min(count - 1) as f64
        } else {
            1.
        },
    )?;
    let alpha = o.nums(e, n, "alpha", 0.05)?;
    let dof = o.nums(e, n, "dof", f64::NAN)?;
    let mut h = vec![];
    let mut p = vec![];
    let mut stat = vec![];
    let mut cv = vec![];
    for i in 0..o.count {
        let lag = integer(n, *at(&lags, i), 1, (count - 1).min(MAX_LAGS))?;
        let alpha = *at(&alpha, i);
        if !alpha.is_finite() || !(0. < alpha && alpha < 1.) {
            return Err(err(
                n,
                "InvalidAlpha",
                "Alpha must lie strictly between 0 and 1.",
            ));
        }
        let df = if n == "lbqtest" && o.values.contains_key("dof") {
            integer(n, *at(&dof, i), 1, lag)?
        } else {
            lag
        };
        let s = if n == "lbqtest" {
            let r = correlogram::acf(e, n, y.data(), lag)?;
            count as f64
                * (count + 2) as f64
                * (1..=lag)
                    .map(|j| r[j] * r[j] / (count - j) as f64)
                    .sum::<f64>()
        } else {
            let rows = y.len() - lag;
            if lag >= count - 2 || rows <= lag + 1 {
                return Err(err(
                    n,
                    "TooFewObservations",
                    "Too few observations for the requested ARCH regression.",
                ));
            }
            if rows.saturating_mul(lag + 1) > MAX_DESIGN {
                return Err(err(
                    n,
                    "WorkspaceLimit",
                    "ARCH regression design exceeds 2000000 elements.",
                ));
            }
            design(e, n, rows, lag + 1)?;
            let x = DMatrix::from_fn(rows, lag + 1, |r, c| {
                if c == 0 {
                    1.
                } else {
                    y.data()[r + lag - c].powi(2)
                }
            });
            let sq = y.data()[lag..].iter().map(|v| v * v).collect::<Vec<_>>();
            let fit = regression::fit(e, n, &x, &sq, true)?;
            rows as f64 * (1. - fit.sse / fit.sst).max(0.)
        };
        let prob = Family::Chi2.cdf(s, &[df as f64], true);
        let c = Family::Chi2.inv(1. - alpha, &[df as f64]);
        h.push(s > c);
        p.push(prob);
        stat.push(s);
        cv.push(c);
    }
    test_values(o.row, h, p, stat, cv)
}
