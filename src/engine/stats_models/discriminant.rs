//! Gaussian discriminants: sample covariance, log posteriors, spectral solves.
use super::*;
use nalgebra::{DVector, SymmetricEigen};
use std::sync::Arc;
#[derive(Clone, Debug)]
pub(in crate::engine) struct Discriminant {
    mu: DMatrix<f64>,
    sigma: Vec<DMatrix<f64>>,
    factors: Vec<DMatrix<f64>>,
    logdet: Vec<f64>,
    prior: Vec<f64>,
    quadratic: bool,
    diagonal: bool,
}
impl Discriminant {
    pub(in crate::engine) fn priors(&self) -> &[f64] {
        &self.prior
    }
    pub(in crate::engine) fn bytes(&self) -> usize {
        128 + 8
            * (self.mu.len()
                + self.sigma.iter().map(|m| m.len()).sum::<usize>()
                + self.factors.iter().map(|m| m.len()).sum::<usize>()
                + self.prior.len() * 2)
    }
    pub(in crate::engine) fn predict(&self, e: &Engine, x: &Matrix) -> Result<Matrix> {
        let n = x.rows();
        let k = self.prior.len();
        let mut scores = vec![0.; size(e, "predict", n, k)?];
        for i in 0..n {
            e.check_cancel()?;
            let mut logs = vec![0.; k];
            for (c, log) in logs.iter_mut().enumerate() {
                let f = if self.quadratic { c } else { 0 };
                let delta = DVector::from_iterator(
                    x.cols(),
                    (0..x.cols()).map(|j| x.data()[i + j * n] - self.mu[(c, j)]),
                );
                let z = self.factors[f].transpose() * delta;
                *log = self.prior[c].ln() - 0.5 * (self.logdet[f] + z.norm_squared());
            }
            let max = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let sum = logs.iter().map(|l| (l - max).exp()).sum::<f64>();
            for c in 0..k {
                scores[i + c * n] = (logs[c] - max).exp() / sum;
            }
        }
        Matrix::from_column_vec(n, k, scores)
    }
    pub(in crate::engine) fn property(&self, e: &Engine, name: &str) -> Result<Value> {
        let k = self.prior.len();
        let p = self.mu.ncols();
        match name {
            "Mu" => matrix_value(self.mu.clone()),
            "Prior" => value(1, k, self.prior.clone()),
            "Sigma" => {
                let data: Vec<_> = self
                    .sigma
                    .iter()
                    .flat_map(|s| {
                        if self.diagonal {
                            (0..p).map(|j| s[(j, j)]).collect()
                        } else {
                            s.as_slice().to_vec()
                        }
                    })
                    .collect();
                if self.quadratic {
                    super::partition::nd_value(
                        e,
                        if self.diagonal {
                            vec![1, p, k]
                        } else {
                            vec![p, p, k]
                        },
                        data,
                    )
                } else {
                    value(if self.diagonal { 1 } else { p }, p, data)
                }
            }
            "Coeffs" => {
                size(e, "fitcdiscr Coeffs", k * k, p * p + p + 1)?;
                let mut slots = Vec::new();
                let fields: Vec<String> = if self.quadratic {
                    vec!["Const", "Linear", "Quadratic"]
                } else {
                    vec!["Const", "Linear"]
                }
                .into_iter()
                .map(String::from)
                .collect();
                // The boundary is log P(class i|x) - log P(class j|x).
                for j in 0..k {
                    for i in 0..k {
                        e.check_cancel()?;
                        let fi = if self.quadratic { i } else { 0 };
                        let fj = if self.quadratic { j } else { 0 };
                        let mi = self.mu.row(i).transpose();
                        let mj = self.mu.row(j).transpose();
                        let ai = &self.factors[fi] * (self.factors[fi].transpose() * &mi);
                        let aj = &self.factors[fj] * (self.factors[fj].transpose() * &mj);
                        let constant = (self.prior[i] / self.prior[j]).ln()
                            - 0.5 * (self.logdet[fi] - self.logdet[fj] + mi.dot(&ai) - mj.dot(&aj));
                        slots.push(Arc::new(Array::Numeric(Matrix::scalar(constant))));
                        slots.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                            p,
                            1,
                            (ai - aj).as_slice().to_vec(),
                        )?)));
                        if self.quadratic {
                            let q = (&self.factors[fj] * self.factors[fj].transpose()
                                - &self.factors[fi] * self.factors[fi].transpose())
                                * 0.5;
                            slots.push(Arc::new(Array::Numeric(Matrix::from_column_vec(
                                p,
                                p,
                                q.as_slice().to_vec(),
                            )?)));
                        }
                    }
                }
                Ok(Value::Container(Array::Struct(
                    crate::StructArray::from_slots(k, k, fields, slots, &e.cancel)?,
                )))
            }
            _ => Err(format!(
                "ClassificationDiscriminant: unknown property '{name}'"
            )),
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn fit(
    e: &Engine,
    x: &Matrix,
    codes: &[usize],
    classes: usize,
    kind: &str,
    gamma: f64,
    prior: Option<&[f64]>,
    uniform: bool,
) -> Result<Discriminant> {
    let n = x.rows();
    let p = x.cols();
    let quadratic = kind.contains("quadratic");
    let diagonal = kind.starts_with("diag");
    let pseudo = kind.starts_with("pseudo");
    if (n as u128) * (p as u128) * (p as u128) > 100_000_000
        || classes.saturating_mul(p * p) > 4_000_000
    {
        return Err("fitcdiscr: covariance workspace or work limit exceeded".into());
    }
    size(e, "fitcdiscr", classes * p, p * 3)?;
    let mut counts = vec![0usize; classes];
    let mut mu = DMatrix::zeros(classes, p);
    for (i, &c) in codes.iter().enumerate() {
        counts[c] += 1;
        for j in 0..p {
            mu[(c, j)] += x.data()[i + j * n];
        }
    }
    for c in 0..classes {
        for j in 0..p {
            mu[(c, j)] /= counts[c] as f64;
        }
    }
    let mut sigma = vec![DMatrix::zeros(p, p); if quadratic { classes } else { 1 }];
    for (i, &c) in codes.iter().enumerate() {
        if i.is_multiple_of(64) {
            e.check_cancel()?;
        }
        let s = if quadratic { c } else { 0 };
        for j in 0..p {
            for l in 0..=j {
                let z = (x.data()[i + j * n] - mu[(c, j)]) * (x.data()[i + l * n] - mu[(c, l)]);
                sigma[s][(j, l)] += z;
                if j != l {
                    sigma[s][(l, j)] += z;
                }
            }
        }
    }
    for (s, cov) in sigma.iter_mut().enumerate() {
        let dof = if quadratic {
            counts[s].saturating_sub(1)
        } else {
            n.saturating_sub(classes)
        };
        if dof == 0 {
            return Err("fitcdiscr: insufficient observations to estimate covariance".into());
        }
        *cov /= dof as f64;
        for j in 0..p {
            for l in 0..p {
                if j != l {
                    cov[(j, l)] *= if diagonal { 0. } else { 1. - gamma };
                }
            }
        }
    }
    let prior = if let Some(v) = prior {
        if v.len() != classes
            || v.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !v.iter().sum::<f64>().is_finite()
        {
            return Err("fitcdiscr: Prior must have one finite positive value per class".into());
        }
        let sum = v.iter().sum::<f64>();
        v.iter().map(|v| v / sum).collect()
    } else if uniform {
        vec![1. / classes as f64; classes]
    } else {
        counts.iter().map(|&c| c as f64 / n as f64).collect()
    };
    let mut factors = Vec::new();
    let mut logdet = Vec::new();
    for cov in &mut sigma {
        e.check_cancel()?;
        if cov.iter().any(|z| !z.is_finite()) {
            return Err("fitcdiscr: numerical overflow in covariance".into());
        }
        let mut eig = SymmetricEigen::new(cov.clone());
        let max = eig.eigenvalues.iter().copied().fold(0., f64::max);
        let tol = (max * f64::EPSILON * p as f64 * 10.).max(f64::MIN_POSITIVE);
        if !pseudo && eig.eigenvalues.iter().any(|&v| v <= tol) {
            if quadratic || diagonal {
                return Err("fitcdiscr: singular covariance; use a pseudo discriminant".into());
            }
            // Minimal diagonal shrinkage for singular pooled covariance.
            let mut shrink = 1e-12;
            loop {
                let mut adjusted = cov.clone();
                for j in 0..p {
                    for l in 0..p {
                        if j != l {
                            adjusted[(j, l)] *= 1. - shrink;
                        }
                    }
                }
                eig = SymmetricEigen::new(adjusted.clone());
                if eig.eigenvalues.iter().all(|&v| v > tol) {
                    *cov = adjusted;
                    break;
                }
                shrink *= 10.;
                if shrink > 1. {
                    return Err("fitcdiscr: zero variance predictor; use pseudoLinear".into());
                }
            }
        }
        let mut factor = eig.eigenvectors.clone();
        let mut log = 0.;
        for j in 0..p {
            let v = eig.eigenvalues[j];
            if v > tol {
                log += v.ln();
                factor.column_mut(j).scale_mut(1. / v.sqrt());
            } else {
                factor.column_mut(j).fill(0.);
            }
        }
        factors.push(factor);
        logdet.push(log);
    }
    Ok(Discriminant {
        mu,
        sigma,
        factors,
        logdet,
        prior,
        quadratic,
        diagonal,
    })
}
