//! Additional real polyfit outputs. Coefficients retain the established
//! column-pivoted QR solve; S.R uses the unpivoted QR of the same design.
use super::{DMatrix, Engine, Matrix, QR, Result, Value};
use crate::{
    Array, StructArray,
    matrix::{Dispersion, StatisticsAxis},
};
use std::sync::Arc;

pub(super) fn outputs(engine: &mut Engine, args: &[Value], count: usize) -> Result<Vec<Value>> {
    if count > 3 {
        return Err("polyfit supports at most three outputs".into());
    }
    Engine::arity_check("polyfit", args, &[3])?;
    let original_x = engine.finite_square_or_matrix("polyfit", &args[0])?;
    let y = engine.finite_square_or_matrix("polyfit", &args[1])?;
    if original_x.len() != y.len()
        || (original_x.rows() != 1 && original_x.cols() != 1)
        || (y.rows() != 1 && y.cols() != 1)
    {
        return Err("polyfit: X and Y must be vectors of the same length".into());
    }
    let mu = if count == 3 {
        let mean = original_x.sum_along(None, true, &engine.cancel)?.data()[0];
        let scale = original_x
            .dispersion(
                Dispersion::StandardDeviation,
                false,
                StatisticsAxis::All,
                false,
                &engine.cancel,
            )?
            .data()[0];
        if !mean.is_finite() || !scale.is_finite() || scale <= 0. {
            return Err("polyfit: centering and scaling require a finite mean and positive standard deviation".into());
        }
        Some([mean, scale])
    } else {
        None
    };
    let x = if let Some([mean, scale]) = mu {
        Matrix::from_column_vec(
            original_x.rows(),
            original_x.cols(),
            original_x
                .data()
                .iter()
                .map(|v| (v - mean) / scale)
                .collect(),
        )?
    } else {
        original_x
    };
    let fit_args = [
        Value::Matrix(x.clone()),
        Value::Matrix(y.clone()),
        args[2].clone(),
    ];
    // Existing validation, admission limits and the proven solve stay shared.
    let mut values = engine.polyfit_call(&fit_args, 1)?;
    let Value::Matrix(coeff) = &values[0] else {
        return Err("polyfit: expected real coefficients".into());
    };
    let terms = coeff.len();
    if x.len() < terms {
        return Err("polyfit: statistics require at least degree+1 points".into());
    }
    engine.preflight_elements(x.len() * terms)?;
    let at = crate::array::evaluate_polynomial(coeff, &x, &engine.cancel)?;
    let norm = y
        .data()
        .iter()
        .zip(at.data())
        .fold(0_f64, |norm, (y, p)| norm.hypot(y - p));
    let mut columns = vec![0.; x.len() * terms];
    for (row, v) in x.data().iter().enumerate() {
        if row.is_multiple_of(1024) {
            engine.check_cancel()?;
        }
        let mut power = 1.;
        for col in (0..terms).rev() {
            columns[row + col * x.len()] = power;
            power *= v;
        }
    }
    engine.check_cancel()?;
    let r = QR::new(DMatrix::from_vec(x.len(), terms, columns)).r();
    engine.check_cancel()?;
    let fields = [
        (
            "R",
            Matrix::from_column_slice(r.nrows(), r.ncols(), r.as_slice())?,
        ),
        ("df", Matrix::scalar((x.len() - terms) as f64)),
        ("normr", Matrix::scalar(norm)),
    ];
    let names = fields.iter().map(|(name, _)| (*name).to_owned()).collect();
    let slots = fields
        .into_iter()
        .map(|(_, m)| Arc::new(Array::Numeric(m)))
        .collect();
    values.push(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        names,
        slots,
        &engine.cancel,
    )?)));
    if let Some(mu) = mu {
        values.push(Value::Matrix(Matrix::from_column_slice(1, 2, &mu)?));
    }
    Ok(values)
}
