//! Weighted PCA by SVD or symmetric eigendecomposition of the covariance.
//! Jolliffe, Principal Component Analysis (2nd ed., 2002), chapters 1 and 14.
use super::*;

pub(super) fn call(e: &Engine, name: &str, a: &[Value]) -> Result<Vec<Value>> {
    if a.is_empty() {
        return Err(format!("{name}: expected a data matrix"));
    }
    let x = real(e, name, &a[0])?;
    if x.cols() == 0 {
        return Err(format!("{name}: empty variables"));
    }
    size(e, name, x.cols(), x.cols())?;
    if name == "pcacov" {
        if a.len() != 1 || x.rows() != x.cols() || x.data().iter().any(|v| !v.is_finite()) {
            return Err("pcacov: expected a finite square covariance matrix".into());
        }
        let cov = dense(&x);
        check_symmetric(name, &cov)?;
        let (coeff, latent) = eigen(cov)?;
        let total = latent.iter().sum::<f64>();
        let explained = latent.iter().map(|v| 100. * v / total).collect();
        return Ok(vec![
            matrix_value(coeff)?,
            value(x.cols(), 1, latent)?,
            value(x.cols(), 1, explained)?,
        ]);
    }
    let mut centered = true;
    let mut economy = true;
    let mut components = None;
    let mut rows = "complete".to_string();
    let mut algorithm = "svd".to_string();
    let mut weights = vec![1.; x.rows()];
    let mut variable = vec![1.; x.cols()];
    let mut variance = false;
    for (key, v) in options(
        name,
        &a[1..],
        &[
            "centered",
            "economy",
            "numcomponents",
            "rows",
            "algorithm",
            "weights",
            "variableweights",
        ],
    )? {
        match key.as_str() {
            "centered" => centered = boolean(e, name, v)?,
            "economy" => economy = boolean(e, name, v)?,
            "numcomponents" => components = Some(integer(e, name, v)?),
            "rows" => rows = text(v).ok_or("pca: Rows must be text")?.to_lowercase(),
            "algorithm" => algorithm = text(v).ok_or("pca: Algorithm must be text")?.to_lowercase(),
            "weights" => {
                let w = real(e, name, v)?;
                if w.len() != x.rows() {
                    return Err("pca: Weights must match rows".into());
                }
                weights = w.data().to_vec();
            }
            _ => {
                if let Some(s) = text(v) {
                    if !s.eq_ignore_ascii_case("variance") {
                        return Err(
                            "pca: VariableWeights must be variance or a positive vector".into()
                        );
                    }
                    variance = true;
                } else {
                    let w = real(e, name, v)?;
                    if w.len() != x.cols() {
                        return Err("pca: VariableWeights must match columns".into());
                    }
                    variable = w.data().to_vec();
                }
            }
        }
    }
    if rows == "pairwise" {
        return Err("pca: Rows pairwise is not supported yet".into());
    }
    if !["complete", "all"].contains(&rows.as_str())
        || !["svd", "eig"].contains(&algorithm.as_str())
    {
        return Err("pca: unsupported Rows or Algorithm".into());
    }
    if weights.iter().any(|v| !v.is_finite() || *v <= 0.)
        || variable.iter().any(|v| !v.is_finite() || *v <= 0.)
    {
        return Err("pca: weights must be finite and positive".into());
    }
    let keep = clean_rows(e, name, &x, None)?;
    let (n, p) = (keep.len(), x.cols());
    let dof = n.saturating_sub(usize::from(centered));
    if dof == 0 {
        return Err("pca: too few complete observations".into());
    }
    if rows == "all" && n != x.rows() {
        return Err("pca: Rows all requires data without NaN".into());
    }
    let total = keep.iter().map(|&r| weights[r]).sum::<f64>();
    // Normalize observation weights to mean 1; weighted sample divisor is
    // sum(w)-sum(w^2)/sum(w), or sum(w) for uncentered PCA.
    let w: Vec<_> = keep
        .iter()
        .map(|&r| weights[r] * n as f64 / total)
        .collect();
    let divisor = if centered {
        n as f64 - w.iter().map(|v| v * v).sum::<f64>() / n as f64
    } else {
        n as f64
    };
    let mut mu = vec![0.; p];
    if centered {
        for (j, mean) in mu.iter_mut().enumerate() {
            *mean = (0..n)
                .map(|i| w[i] * x.data()[keep[i] + j * x.rows()])
                .sum::<f64>()
                / n as f64;
        }
    }
    if variance {
        if n < 2 {
            return Err("pca: variance weights require at least two observations".into());
        }
        for (j, weight) in variable.iter_mut().enumerate() {
            let mean = (0..n)
                .map(|i| w[i] * x.data()[keep[i] + j * x.rows()])
                .sum::<f64>()
                / n as f64;
            let var = (0..n)
                .map(|i| w[i] * (x.data()[keep[i] + j * x.rows()] - mean).powi(2))
                .sum::<f64>()
                / (n as f64 - w.iter().map(|v| v * v).sum::<f64>() / n as f64);
            if !var.is_finite() || var <= 0. {
                return Err("pca: variance weights require nonconstant variables".into());
            }
            *weight = 1. / var;
        }
    }
    let scaled = DMatrix::from_fn(n, p, |i, j| {
        (x.data()[keep[i] + j * x.rows()] - mu[j]) * variable[j].sqrt() * w[i].sqrt()
    });
    if scaled.iter().any(|v| !v.is_finite()) {
        return Err("pca: nonfinite values produced during scaling".into());
    }
    e.check_cancel()?;
    let (mut coeff, mut latent) = if algorithm == "eig" || (!economy && n < p) {
        eigen(scaled.transpose() * &scaled / divisor)?
    } else {
        let svd = scaled.svd(false, true);
        let vt = svd.v_t.ok_or("pca: SVD did not return vectors")?;
        let mut order: Vec<_> = (0..svd.singular_values.len()).collect();
        order.sort_by(|&i, &j| svd.singular_values[j].total_cmp(&svd.singular_values[i]));
        (
            DMatrix::from_fn(p, order.len(), |r, c| vt[(order[c], r)]),
            order
                .iter()
                .map(|&i| svd.singular_values[i].powi(2) / divisor)
                .collect(),
        )
    };
    e.check_cancel()?;
    let totalvar = latent.iter().sum::<f64>();
    let available = if economy { dof.min(p) } else { p };
    let q = components.unwrap_or(available);
    if q > available {
        return Err("pca: NumComponents exceeds available components".into());
    }
    coeff = coeff.columns(0, available).into_owned();
    for j in 0..available {
        for i in 0..p {
            coeff[(i, j)] /= variable[i].sqrt();
        }
    }
    sign_columns(&mut coeff);
    latent.truncate(available);
    let mut score = vec![f64::NAN; x.rows() * q];
    // Scores use all retained coefficients; T-squared uses all nonzero
    // components, including those omitted by NumComponents.
    let mut tsquared = vec![f64::NAN; x.rows()];
    for (i, &r) in keep.iter().enumerate() {
        if i.is_multiple_of(256) {
            e.check_cancel()?;
        }
        let mut t = 0.;
        for j in 0..available {
            let s = (0..p)
                .map(|c| (x.data()[r + c * x.rows()] - mu[c]) * variable[c] * coeff[(c, j)])
                .sum::<f64>();
            if j < q {
                score[r + j * x.rows()] = s;
            }
            if latent[j] > f64::EPSILON * latent[0] * p as f64 {
                t += s * s / latent[j];
            }
        }
        tsquared[r] = t;
    }
    let explained = latent.iter().map(|v| 100. * v / totalvar).collect();
    Ok(vec![
        matrix_value(coeff.columns(0, q).into_owned())?,
        value(x.rows(), q, score)?,
        value(available, 1, latent)?,
        value(x.rows(), 1, tsquared)?,
        value(available, 1, explained)?,
        value(1, p, mu)?,
    ])
}

fn check_symmetric(name: &str, a: &DMatrix<f64>) -> Result<()> {
    let scale = a.iter().fold(1_f64, |m, v| m.max(v.abs()));
    for i in 0..a.nrows() {
        for j in 0..i {
            if (a[(i, j)] - a[(j, i)]).abs() > scale * 1e-12 {
                return Err(format!("{name}: expected a symmetric covariance matrix"));
            }
        }
    }
    Ok(())
}
fn eigen(a: DMatrix<f64>) -> Result<(DMatrix<f64>, Vec<f64>)> {
    if a.iter().any(|v| !v.is_finite()) {
        return Err("pcacov/pca: nonfinite covariance".into());
    }
    let eig = a.symmetric_eigen();
    let mut order: Vec<_> = (0..eig.eigenvalues.len()).collect();
    order.sort_by(|&i, &j| eig.eigenvalues[j].total_cmp(&eig.eigenvalues[i]));
    let mut coeff = DMatrix::from_fn(order.len(), order.len(), |r, c| {
        eig.eigenvectors[(r, order[c])]
    });
    let scale = eig.eigenvalues.iter().fold(1_f64, |m, v| m.max(v.abs()));
    if eig.eigenvalues.iter().any(|v| *v < -scale * 1e-12) {
        return Err("pcacov/pca: covariance must be positive semidefinite".into());
    }
    let latent = order.iter().map(|&i| eig.eigenvalues[i].max(0.)).collect();
    sign_columns(&mut coeff);
    Ok((coeff, latent))
}
fn sign_columns(coeff: &mut DMatrix<f64>) {
    for j in 0..coeff.ncols() {
        let mut index = 0;
        for i in 1..coeff.nrows() {
            if coeff[(i, j)].abs() > coeff[(index, j)].abs() {
                index = i;
            }
        }
        if coeff[(index, j)] < 0. {
            coeff.column_mut(j).scale_mut(-1.);
        }
    }
}
