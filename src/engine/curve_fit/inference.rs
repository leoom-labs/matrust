//! confint and predint, by the formulas of the public "Confidence and
//! Prediction Bounds" page: coefficient bounds b +- t*sqrt(diag(S)) with
//! S = inv(J'J)*mse, and prediction bounds yhat +- c*sqrt(g S g' [+ mse]) with
//! c = t(1-alpha/2, dfe) (non-simultaneous) or the Scheffe-type
//! sqrt(m*F(level; m, dfe)), m = p (functional) or p+1 (observation).
use super::super::stats_dist::families::{f_quantile, t_quantile};
use super::super::{Engine, Execution, Value};
use super::Fitted;
use super::fit::real_matrix;
use super::options::text_of;
use crate::{Matrix, Result};

fn level(e: &Engine, v: Option<&Value>, who: &str) -> Result<f64> {
    let Some(v) = v else {
        return Ok(0.95);
    };
    let m = real_matrix(e, v, who, "the confidence level")?;
    if m.len() != 1 || !(m.data()[0] > 0. && m.data()[0] < 1.) {
        return Err(format!(
            "{who}: the confidence level must be a scalar between 0 and 1"
        ));
    }
    Ok(m.data()[0])
}

/// Two-sided t critical value, NaN without error degrees of freedom.
fn t_crit(level: f64, dfe: f64) -> f64 {
    if dfe > 0. {
        t_quantile((1. + level) / 2., dfe)
    } else {
        f64::NAN
    }
}

pub(super) fn confint(e: &Engine, f: &Fitted, rest: &[Value]) -> Result<Value> {
    if rest.len() > 1 {
        return Err("confint: expected confint(fitobject) or confint(fitobject, level)".into());
    }
    let level = level(e, rest.first(), "confint")?;
    let (lo, hi) = bounds(f, level);
    let mut data = Vec::with_capacity(2 * lo.len());
    for (l, h) in lo.iter().zip(&hi) {
        data.push(*l);
        data.push(*h);
    }
    Ok(Value::Matrix(Matrix::from_column_vec(2, lo.len(), data)?))
}

/// Lower and upper coefficient bounds at `level` (NaN where unknown).
pub(super) fn bounds(f: &Fitted, level: f64) -> (Vec<f64>, Vec<f64>) {
    let p = f.coef.len();
    let t = t_crit(level, f.dfe);
    (0..p)
        .map(|k| {
            let var = f.cov.get(k + k * p).copied().unwrap_or(f64::NAN) * f.mse;
            let half = t * var.sqrt();
            (f.coef[k] - half, f.coef[k] + half)
        })
        .unzip()
}

/// [ci, y] = predint(fitobject, x, level, intopt, simopt)
pub(super) fn predint(
    e: &mut Engine,
    f: &Fitted,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if rest.is_empty() {
        return Err("predint: expected predint(fitobject, x, level, intopt, simopt)".into());
    }
    if rest.len() > 4 {
        return Err("predint: too many input arguments".into());
    }
    let level = level(e, rest.get(1), "predint")?;
    let observation = match rest
        .get(2)
        .map(|v| text_of(v).map(|t| t.to_ascii_lowercase()))
    {
        None => true,
        Some(Some(t)) if t == "observation" => true,
        Some(Some(t)) if t == "functional" => false,
        _ => return Err("predint: intopt must be 'observation' or 'functional'".into()),
    };
    let simultaneous = match rest
        .get(3)
        .map(|v| text_of(v).map(|t| t.to_ascii_lowercase()))
    {
        None => false,
        Some(Some(t)) if t == "on" => true,
        Some(Some(t)) if t == "off" => false,
        _ => return Err("predint: simopt must be 'on' or 'off'".into()),
    };
    let (xs, _, _) = super::query_points(e, &f.ft, &rest[..1], "predint")?;
    let n = xs[0].len();
    let yhat = super::eval_fitted(e, f, &xs, out)?;
    let p = f.coef.len();
    let usable = f.dfe > 0. && f.cov.len() == p * p && f.knots.is_none();
    let half: Vec<f64> = if usable {
        let crit = if simultaneous {
            let m = if observation { p + 1 } else { p } as f64;
            (m * f_quantile(level, m, f.dfe)).sqrt()
        } else {
            t_crit(level, f.dfe)
        };
        let normalized = super::normalize(f, &xs);
        let refs: Vec<&[f64]> = normalized.iter().map(Vec::as_slice).collect();
        let (g, _) = super::model::coefficient_jacobian(
            e,
            &f.ft,
            &f.coef,
            &f.problem,
            &refs,
            None,
            (1e-8, 0.1),
            out,
        )?;
        (0..n)
            .map(|i| {
                let mut var = 0.;
                for a in 0..p {
                    for b in 0..p {
                        var += g[(i, a)] * f.cov[a + b * p] * g[(i, b)];
                    }
                }
                var *= f.mse;
                if observation {
                    var += f.mse;
                }
                crit * var.sqrt()
            })
            .collect()
    } else {
        vec![f64::NAN; n]
    };
    let mut data = Vec::with_capacity(2 * n);
    data.extend(yhat.iter().zip(&half).map(|(y, h)| y - h));
    data.extend(yhat.iter().zip(&half).map(|(y, h)| y + h));
    let mut values = vec![Value::Matrix(Matrix::from_column_vec(n, 2, data)?)];
    if nargout >= 2 {
        values.push(Value::Matrix(Matrix::from_column_vec(n, 1, yhat)?));
    }
    Ok(values)
}
