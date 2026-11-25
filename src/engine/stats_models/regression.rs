//! Pivoted QR least squares and coordinate descent (Friedman, Hastie and
//! Tibshirani, JSS 33(1), 2010, equations 1 and 5).
use super::super::stats_dist::families::Family;
use super::*;
use nalgebra::{DVector, linalg::ColPivQR};
use std::sync::Arc;

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    a: &[Value],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "regress" => regress(e, a, out),
        "ridge" => ridge(e, a),
        _ => lasso(e, a, out),
    }
}

fn inputs(e: &Engine, name: &str, y: &Value, x: &Value) -> Result<(Matrix, Matrix, Vec<usize>)> {
    let y = real(e, name, y)?;
    let x = real(e, name, x)?;
    if y.len() != x.rows() || (y.cols() != 1 && y.rows() != 1) || x.cols() == 0 {
        return Err(format!(
            "{name}: response must be a vector with one entry per predictor row"
        ));
    }
    let keep = clean_rows(e, name, &x, Some(&y))?;
    if keep.is_empty() {
        return Err(format!("{name}: no complete observations"));
    }
    Ok((y, x, keep))
}

fn regress(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if !(2..=3).contains(&a.len()) {
        return Err("regress: expected y, X and optional alpha".into());
    }
    let alpha = if a.len() == 3 {
        scalar(e, "regress", &a[2])?
    } else {
        0.05
    };
    if !(0. ..1.).contains(&alpha) || alpha == 0. {
        return Err("regress: alpha must be in (0,1)".into());
    }
    let (y, x, keep) = inputs(e, "regress", &a[0], &a[1])?;
    let (n, p) = (keep.len(), x.cols());
    size(e, "regress", n, p)?;
    size(e, "regress", p, p)?;
    let design = DMatrix::from_fn(n, p, |r, c| x.data()[keep[r] + c * x.rows()]);
    let response = DVector::from_iterator(n, keep.iter().map(|&r| y.data()[r]));
    let fit = least_squares(e, &design, &response)?;
    let (b, q, rank) = (fit.b, fit.q, fit.rank);
    let covdiag = DVector::from_iterator(
        p,
        (0..p).map(|j| fit.covariance_factor.row(j).norm_squared()),
    );
    if rank < p {
        warning(
            e,
            out,
            "regress: design is rank deficient; dependent columns have zero coefficients",
        )?;
    }
    let residual = response.clone() - design * &b;
    let sse = residual.norm_squared();
    let df = n.saturating_sub(rank);
    let errvar = if df > 0 { sse / df as f64 } else { f64::NAN };
    let t = Family::T.inv(1. - alpha / 2., &[df as f64]);
    let mut bint = vec![0.; p * 2];
    for j in 0..p {
        let se = (errvar * covdiag[j]).sqrt();
        bint[j] = b[j] - t * se;
        bint[j + p] = b[j] + t * se;
    }
    let mut res = vec![f64::NAN; x.rows()];
    let mut rint = vec![f64::NAN; x.rows() * 2];
    let tr = Family::T.inv(1. - alpha / 2., &[df.saturating_sub(1) as f64]);
    for (i, &original) in keep.iter().enumerate() {
        let h: f64 = (0..rank).map(|j| q[(i, j)].powi(2)).sum();
        let one_h = (1. - h).max(0.);
        let deleted = if df > 1 && one_h > 0. {
            ((sse - residual[i].powi(2) / one_h).max(0.) / (df - 1) as f64).sqrt()
        } else {
            f64::NAN
        };
        let half = tr * deleted * one_h.sqrt();
        res[original] = residual[i];
        rint[original] = residual[i] - half;
        rint[original + x.rows()] = residual[i] + half;
    }
    let mean = response.sum() / n as f64;
    let sst: f64 = response.iter().map(|v| (v - mean).powi(2)).sum();
    let r2 = 1. - sse / sst;
    let model_df = rank.saturating_sub(1) as f64;
    let f = (sst - sse) / model_df / errvar;
    let prob = Family::F.cdf(f, &[model_df, df as f64], true);
    Ok(vec![
        matrix_value(DMatrix::from_column_slice(p, 1, b.as_slice()))?,
        value(p, 2, bint)?,
        value(x.rows(), 1, res)?,
        value(x.rows(), 2, rint)?,
        value(1, 4, vec![r2, f, prob, errvar])?,
    ])
}

/// Shared column-pivoted QR kernel. The unscaled covariance factor F is in
/// original predictor order: covariance = F F'. Keeping the factor avoids a
/// full matrix product when regress only needs its diagonal. Solve triangular
/// systems, never form or invert the normal equations.
pub(in crate::engine) struct LeastSquares {
    pub b: DVector<f64>,
    pub q: DMatrix<f64>,
    pub covariance_factor: DMatrix<f64>,
    pub rank: usize,
}
pub(in crate::engine) fn least_squares(
    e: &Engine,
    design: &DMatrix<f64>,
    response: &DVector<f64>,
) -> Result<LeastSquares> {
    e.check_cancel()?;
    let (n, p) = design.shape();
    let qr = ColPivQR::new(design.clone());
    e.check_cancel()?;
    let q = qr.q();
    let rmat = qr.r();
    let scale = if n.min(p) > 0 { rmat[(0, 0)].abs() } else { 0. };
    let tol = n.max(p) as f64 * f64::EPSILON * scale;
    let rank = (0..n.min(p))
        .take_while(|&i| rmat[(i, i)].abs() > tol)
        .count();
    let projected = q.transpose() * response;
    let mut b = DVector::zeros(p);
    let mut covariance_factor = DMatrix::zeros(p, rank);
    if rank > 0 {
        let r11 = rmat.view((0, 0), (rank, rank)).into_owned();
        let head = r11
            .solve_upper_triangular(&projected.rows(0, rank).into_owned())
            .ok_or("least squares: QR solve failed")?;
        b.rows_mut(0, rank).copy_from(&head);
        let solved = r11
            .solve_upper_triangular(&DMatrix::identity(rank, rank))
            .ok_or("least squares: QR covariance solve failed")?;
        covariance_factor
            .view_mut((0, 0), (rank, rank))
            .copy_from(&solved);
    }
    qr.p().inv_permute_rows(&mut b);
    qr.p().inv_permute_rows(&mut covariance_factor);
    Ok(LeastSquares {
        b,
        q,
        covariance_factor,
        rank,
    })
}

fn ridge(e: &Engine, a: &[Value]) -> Result<Vec<Value>> {
    if !(3..=4).contains(&a.len()) {
        return Err("ridge: expected y, X, k and optional scaled".into());
    }
    let (y, x, keep) = inputs(e, "ridge", &a[0], &a[1])?;
    let k = real(e, "ridge", &a[2])?;
    if (k.rows() != 1 && k.cols() != 1)
        || k.is_empty()
        || k.data().iter().any(|v| !v.is_finite() || *v < 0.)
    {
        return Err("ridge: k must be a finite nonnegative vector".into());
    }
    let scaled = if a.len() == 4 {
        boolean(e, "ridge", &a[3])?
    } else {
        true
    };
    let (n, p) = (keep.len(), x.cols());
    if n < 2 {
        return Err("ridge: at least two complete observations required".into());
    }
    let mut means = vec![0.; p];
    let mut scales = vec![0.; p];
    let ym = keep.iter().map(|&i| y.data()[i]).sum::<f64>() / n as f64;
    for j in 0..p {
        means[j] = keep
            .iter()
            .map(|&i| x.data()[i + j * x.rows()])
            .sum::<f64>()
            / n as f64;
        scales[j] = (keep
            .iter()
            .map(|&i| (x.data()[i + j * x.rows()] - means[j]).powi(2))
            .sum::<f64>()
            / (n - 1) as f64)
            .sqrt();
        if scales[j] == 0. {
            return Err("ridge: constant predictors cannot be standardized".into());
        }
    }
    size(e, "ridge", n + p, p)?;
    let z = DMatrix::from_fn(n, p, |i, j| {
        (x.data()[keep[i] + j * x.rows()] - means[j]) / scales[j]
    });
    let yc = DVector::from_iterator(n, keep.iter().map(|&i| y.data()[i] - ym));
    let rows = p + usize::from(!scaled);
    size(e, "ridge", rows, k.len())?;
    let mut result = DMatrix::zeros(rows, k.len());
    for (col, &penalty) in k.data().iter().enumerate() {
        e.check_cancel()?;
        let augmented = DMatrix::from_fn(n + p, p, |i, j| {
            if i < n {
                z[(i, j)]
            } else if i - n == j {
                penalty.sqrt()
            } else {
                0.
            }
        });
        let rhs = DVector::from_fn(n + p, |i, _| if i < n { yc[i] } else { 0. });
        let svd = augmented.svd(true, true);
        let tol = f64::EPSILON * (n + p).max(p) as f64 * svd.singular_values[0];
        let b = svd.solve(&rhs, tol).map_err(|s| format!("ridge: {s}"))?;
        let mut intercept = ym;
        for j in 0..p {
            let v = if scaled { b[j] } else { b[j] / scales[j] };
            result[(j + usize::from(!scaled), col)] = v;
            intercept -= means[j] * v;
        }
        if !scaled {
            result[(0, col)] = intercept;
        }
    }
    Ok(vec![matrix_value(result)?])
}

fn lasso(e: &mut Engine, a: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    if a.len() < 2 {
        return Err("lasso: expected X and y".into());
    }
    let (y, x, keep) = inputs(e, "lasso", &a[1], &a[0])?;
    let (n, p) = (keep.len(), x.cols());
    let mut alpha = 1.;
    let mut lambda = None;
    let mut num = 100;
    let mut ratio = 1e-4;
    let mut standard = true;
    let mut intercept = true;
    let mut tol = 1e-4;
    let mut maxiter = 100000;
    let mut weights = vec![1.; x.rows()];
    for (key, v) in options(
        "lasso",
        &a[2..],
        &[
            "alpha",
            "lambda",
            "numlambda",
            "lambdaratio",
            "standardize",
            "intercept",
            "reltol",
            "maxiter",
            "weights",
            "cv",
        ],
    )? {
        match key.as_str() {
            "alpha" => alpha = scalar(e, "lasso", v)?,
            "lambda" => lambda = Some(real(e, "lasso", v)?),
            "numlambda" => num = integer(e, "lasso", v)?,
            "lambdaratio" => ratio = scalar(e, "lasso", v)?,
            "standardize" => standard = boolean(e, "lasso", v)?,
            "intercept" => intercept = boolean(e, "lasso", v)?,
            "reltol" => tol = scalar(e, "lasso", v)?,
            "maxiter" => maxiter = integer(e, "lasso", v)?,
            "weights" => {
                let w = real(e, "lasso", v)?;
                if w.len() != x.rows() {
                    return Err("lasso: Weights must match rows".into());
                }
                weights = w.data().to_vec();
            }
            _ => return Err("lasso: CV is not supported yet".into()),
        }
    }
    if alpha <= 0.
        || alpha > 1.
        || ratio < 0.
        || ratio >= 1.
        || tol <= 0.
        || weights.iter().any(|w| !w.is_finite() || *w < 0.)
    {
        return Err("lasso: invalid Alpha, LambdaRatio, RelTol or Weights".into());
    }
    let total = keep.iter().map(|&i| weights[i]).sum::<f64>();
    if total <= 0. {
        return Err("lasso: Weights must have positive sum".into());
    }
    let w: Vec<_> = keep.iter().map(|&i| weights[i] / total).collect();
    let ym = if intercept {
        keep.iter()
            .enumerate()
            .map(|(i, &r)| w[i] * y.data()[r])
            .sum()
    } else {
        0.
    };
    let mut means = vec![0.; p];
    let mut scales = vec![1.; p];
    size(e, "lasso", n, p)?;
    let mut z = DMatrix::zeros(n, p);
    for j in 0..p {
        let mean = keep
            .iter()
            .enumerate()
            .map(|(i, &r)| w[i] * x.data()[r + j * x.rows()])
            .sum::<f64>();
        means[j] = if intercept { mean } else { 0. };
        if standard {
            scales[j] = keep
                .iter()
                .enumerate()
                .map(|(i, &r)| w[i] * (x.data()[r + j * x.rows()] - mean).powi(2))
                .sum::<f64>()
                .sqrt();
            if scales[j] == 0. {
                scales[j] = 1.;
            }
        }
        for i in 0..n {
            z[(i, j)] = (x.data()[keep[i] + j * x.rows()] - means[j]) / scales[j];
        }
    }
    let yc: Vec<_> = keep.iter().map(|&r| y.data()[r] - ym).collect();
    let mut lambdas = if let Some(l) = lambda {
        if l.is_empty()
            || (l.rows() != 1 && l.cols() != 1)
            || l.data().iter().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err("lasso: Lambda must be a nonnegative finite vector".into());
        }
        l.data().to_vec()
    } else {
        size(e, "lasso", p, num)?;
        let largest = (0..p)
            .map(|j| (0..n).map(|i| w[i] * z[(i, j)] * yc[i]).sum::<f64>().abs() / alpha)
            .fold(0., f64::max);
        (0..num)
            .map(|i| {
                if ratio == 0. && i == num - 1 {
                    0.
                } else {
                    largest
                        * (if ratio == 0. { 1e-4_f64 } else { ratio }).powf(if num == 1 {
                            0.
                        } else {
                            i as f64 / (num - 1) as f64
                        })
                }
            })
            .collect()
    };
    lambdas.sort_by(f64::total_cmp);
    lambdas.dedup();
    let l = lambdas.len();
    size(e, "lasso", p, l)?;
    let mut bs = vec![0.; p * l];
    let mut ints = vec![0.; l];
    let mut mse = vec![0.; l];
    let mut df = vec![0.; l];
    let norms: Vec<f64> = (0..p)
        .map(|j| (0..n).map(|i| w[i] * z[(i, j)].powi(2)).sum())
        .collect();
    let mut b = vec![0.; p];
    let mut residual = yc.clone();
    for col in (0..l).rev() {
        let penalty = lambdas[col];
        let mut converged = false;
        for _ in 0..maxiter {
            e.check_cancel()?;
            let mut change: f64 = 0.;
            let mut magnitude: f64 = 0.;
            for j in 0..p {
                if j.is_multiple_of(256) {
                    e.check_cancel()?;
                }
                let rho =
                    (0..n).map(|i| w[i] * z[(i, j)] * residual[i]).sum::<f64>() + norms[j] * b[j];
                let denom = norms[j] + penalty * (1. - alpha);
                let next = if denom > 0. {
                    rho.signum() * (rho.abs() - penalty * alpha).max(0.) / denom
                } else {
                    0.
                };
                let delta = next - b[j];
                b[j] = next;
                for i in 0..n {
                    residual[i] -= z[(i, j)] * delta;
                }
                change = change.max(delta.abs());
                magnitude = magnitude.max(next.abs());
            }
            if change <= tol * magnitude.max(1.) {
                converged = true;
                break;
            }
        }
        if !converged {
            warning(e, out, "lasso: MaxIter reached before convergence")?;
        }
        ints[col] = ym;
        for j in 0..p {
            bs[j + col * p] = b[j] / scales[j];
            ints[col] -= means[j] * bs[j + col * p];
            if b[j] != 0. {
                df[col] += 1.;
            }
        }
        mse[col] = (0..n).map(|i| w[i] * residual[i].powi(2)).sum();
    }
    let fields = vec![
        "Intercept",
        "Lambda",
        "Alpha",
        "DF",
        "MSE",
        "PredictorNames",
        "UseCovariance",
    ];
    let empty = crate::CellArray::empty(0, 0, &e.cancel)?;
    let slots = vec![
        Array::Numeric(Matrix::from_column_vec(1, l, ints)?),
        Array::Numeric(Matrix::from_column_vec(1, l, lambdas)?),
        Array::Numeric(Matrix::scalar(alpha)),
        Array::Numeric(Matrix::from_column_vec(1, l, df)?),
        Array::Numeric(Matrix::from_column_vec(1, l, mse)?),
        Array::Cell(empty),
        Array::Logical(crate::LogicalMatrix::scalar(false)),
    ];
    let info = crate::StructArray::from_slots(
        1,
        1,
        fields.into_iter().map(str::to_string).collect(),
        slots.into_iter().map(Arc::new).collect(),
        &e.cancel,
    )?;
    Ok(vec![
        value(p, l, bs)?,
        Value::Container(Array::Struct(info)),
    ])
}
