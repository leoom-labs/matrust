//! One-versus-one/all coding with loss-weighted decoding.
use super::*;
use crate::engine::classifier_objects::{Classifier, FitOptions, Labels, predict_rows, take_rows};
use std::sync::Arc;
pub(super) fn fit(
    e: &mut Engine,
    x: &Matrix,
    codes: &[usize],
    classes: usize,
    ova: bool,
    learner: &FitOptions,
) -> Result<super::ml::Model> {
    if classes < 2 {
        return Err("fitcecoc: at least two response classes are required".into());
    }
    let columns = if ova {
        classes
    } else {
        classes * (classes - 1) / 2
    };
    if columns > 512 || columns.saturating_mul(x.len()) > 8_000_000 {
        return Err("fitcecoc: retained binary learner workspace limit exceeded".into());
    }
    let probability = matches!(
        learner,
        FitOptions::Knn { .. } | FitOptions::Tree { .. } | FitOptions::Nb
    ) || matches!(learner,FitOptions::Advanced(c) if matches!(c.as_ref(),super::ml::Config::Discriminant{..}|super::ml::Config::Linear{logistic:true,..}));
    let mut coding = vec![Vec::new(); classes];
    let mut learners = Vec::new();
    let mut bytes = 0usize;
    for first in 0..classes {
        for second in if ova {
            first..first + 1
        } else {
            first + 1..classes
        } {
            e.check_cancel()?;
            let column: Vec<_> = (0..classes)
                .map(|k| {
                    if k == first {
                        1.
                    } else if ova || k == second {
                        -1.
                    } else {
                        0.
                    }
                })
                .collect();
            let ids: Vec<_> = (0..x.rows()).filter(|&i| column[codes[i]] != 0.).collect();
            let y = Labels::Numeric(ids.iter().map(|&i| column[codes[i]]).collect());
            let m = super::classifiers::train(
                e,
                take_rows(x, &ids)?,
                y,
                (1..=x.cols()).map(|j| format!("x{j}")).collect(),
                "Y".into(),
                learner.clone(),
            )?;
            bytes = bytes.saturating_add(m.bytes());
            if bytes > 128 * 1024 * 1024 {
                return Err("fitcecoc: retained binary models exceed 128 MiB".into());
            }
            e.preflight_elements(bytes.div_ceil(8))?;
            learners.push(Arc::new(m));
            for k in 0..classes {
                coding[k].push(column[k]);
            }
        }
    }
    Ok(super::ml::Model::Ecoc {
        learners,
        coding,
        probability,
    })
}
pub(super) fn scores(
    e: &Engine,
    learners: &[Arc<Classifier>],
    coding: &[Vec<f64>],
    probability: bool,
    x: &Matrix,
) -> Result<Matrix> {
    let n = x.rows();
    let k = coding.len();
    let mut result = vec![0.; size(e, "predict", n, k)?];
    let counts: Vec<_> = coding
        .iter()
        .map(|r| r.iter().filter(|&&v| v != 0.).count() as f64)
        .collect();
    for (b, m) in learners.iter().enumerate() {
        let (_, s) = predict_rows(e, m, x)?;
        if s.cols() < 2 {
            return Err("predict: binary learner has one response class".into());
        }
        for c in 0..k {
            let sign = coding[c][b];
            if sign == 0. {
                continue;
            }
            for i in 0..n {
                let score = s.data()[i + n];
                let loss = if probability {
                    (1. - sign * (2. * score - 1.)).powi(2) / 2.
                } else {
                    (1. - sign * score).max(0.) / 2.
                };
                result[i + c * n] -= loss / counts[c];
            }
        }
    }
    Matrix::from_column_vec(n, k, result)
}
