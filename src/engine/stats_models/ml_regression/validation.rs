//! Regression refits use the shared cvpartition engine without response stratification.
use super::super::super::classifier_objects::take_rows;
use super::*;
pub(super) fn crossval(e: &mut Engine, m: &Model, args: &[Value]) -> Result<Value> {
    if matches!(m.kind, Kind::Partitioned { .. }) {
        return Err("crossval: expected an unpartitioned trained model".into());
    }
    let partition =
        super::super::partition::for_crossval(e, m.x.rows(), vec![0; m.x.rows()], args)?;
    if partition.n != m.x.rows() {
        return Err("crossval: CVPartition NumObservations does not match the model".into());
    }
    let tests = partition.tests.clone();
    if tests.len() > 1000 || tests.len().saturating_mul(m.x.len()) > 8_000_000 {
        return Err("crossval: retained fold workspace limit exceeded".into());
    }
    e.preflight_elements(tests.len().saturating_mul(m.bytes().div_ceil(8)))?;
    let mut trained = Vec::new();
    let mut retained = m.bytes();
    for test in &tests {
        e.check_cancel()?;
        let mut is_test = vec![false; m.x.rows()];
        for &i in test {
            is_test[i] = true;
        }
        let ids: Vec<_> = (0..m.x.rows())
            .filter(|&i| partition.resub || !is_test[i])
            .collect();
        if ids.is_empty() {
            return Err("crossval: every fold must retain training observations".into());
        }
        let model = train(
            e,
            take_rows(&m.x, &ids)?,
            take_rows(&m.y, &ids)?,
            m.names.clone(),
            m.response.clone(),
            m.options.clone(),
        )
        .map_err(|s| format!("crossval: fold fitting failed: {s}"))?;
        retained = retained.saturating_add(model.bytes());
        if retained > 128 * 1024 * 1024 {
            return Err("crossval: retained model workspace exceeds 128 MiB".into());
        }
        e.preflight_elements(retained.div_ceil(8))?;
        trained.push(Arc::new(model));
    }
    let mut cv = m.clone();
    cv.kind = Kind::Partitioned { trained, tests };
    super::super::super::objects::mlr_new(e, cv)
}
pub(super) fn predictions(e: &Engine, m: &Model) -> Result<Vec<f64>> {
    let Kind::Partitioned { trained, tests } = &m.kind else {
        return Err("kfoldPredict: expected a partitioned regression model".into());
    };
    let mut y = vec![f64::NAN; m.x.rows()];
    for (model, ids) in trained.iter().zip(tests) {
        let p = predict_rows(e, model, &take_rows(&m.x, ids)?)?;
        for (&i, p) in ids.iter().zip(p) {
            y[i] = p;
        }
    }
    Ok(y)
}
pub(super) fn loss(e: &Engine, m: &Model, name: &str, args: &[Value]) -> Result<Value> {
    let (y, p, opts) = match name {
        "loss" => {
            let [x, y, tail @ ..] = args else {
                return Err("loss: expected model, predictors and response".into());
            };
            let (x, _) = super::super::super::classifier_objects::predictor_matrix(
                e,
                x,
                Some(&m.names),
                name,
            )?;
            let y = real(e, name, y)?;
            if y.len() != x.rows() || (y.rows() != 1 && y.cols() != 1) {
                return Err(
                    "loss: X and Y must have the same number of rows; Y must be a vector".into(),
                );
            }
            if y.data().iter().any(|v| v.is_infinite()) {
                return Err("loss: response must not contain Inf".into());
            }
            (y.data().to_vec(), predict_rows(e, m, &x)?, tail)
        }
        "resubLoss" => (m.y.data().to_vec(), predict_rows(e, m, &m.x)?, args),
        _ => (m.y.data().to_vec(), predictions(e, m)?, args),
    };
    for (_, v) in options(name, opts, &["lossfun"])? {
        if opt_text(e, v, name)? != "mse" {
            return Err(format!("{name}: only LossFun mse is supported"));
        }
    }
    let mut sum = 0.;
    let mut count = 0;
    for (a, b) in y.iter().zip(p) {
        if a.is_finite() && b.is_finite() {
            sum += (a - b).powi(2);
            count += 1;
        }
    }
    if count == 0 {
        return Err(format!("{name}: no complete test observations"));
    }
    value(1, 1, vec![sum / count as f64])
}
