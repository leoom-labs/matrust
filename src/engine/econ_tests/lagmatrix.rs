use super::*;
pub(super) fn call(e: &mut Engine, a: &[Value]) -> Result<Vec<Value>> {
    let n = "lagmatrix";
    if a.len() != 2 {
        return Err(err(
            n,
            "InvalidNumInputs",
            "Expected Y and lags; presample, postsample and Shape options are not supported.",
        ));
    }
    let y = real_allow_inf(e, n, &a[0])?;
    let lags = vector(e, n, &a[1])?;
    if lags.len() > MAX_LAGS {
        return Err(err(n, "InvalidLags", "At most 2048 shifts are supported."));
    }
    let shifts = lags
        .data()
        .iter()
        .map(|&v| {
            if !v.is_finite() || v.fract() != 0. || v.abs() > 9_007_199_254_740_991. {
                Err(err(
                    n,
                    "InvalidLags",
                    "Shifts must be finite integers with magnitude below flintmax.",
                ))
            } else {
                Ok(v as i64)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let cols = y
        .cols()
        .checked_mul(shifts.len())
        .ok_or("lagmatrix dimensions overflow")?;
    let count = y
        .rows()
        .checked_mul(cols)
        .ok_or("lagmatrix dimensions overflow")?;
    if count > MAX_DESIGN {
        return Err(err(n, "WorkspaceLimit", "Output exceeds 2000000 elements."));
    }
    reserve(e, n, count.saturating_add(y.rows()))?;
    e.charge(count)?;
    let mut data = vec![f64::NAN; count];
    for (j, &lag) in shifts.iter().enumerate() {
        for col in 0..y.cols() {
            e.check_cancel()?;
            for i in 0..y.rows() {
                let source = i as i64 - lag;
                if source >= 0 && source < y.rows() as i64 {
                    data[i + (j * y.cols() + col) * y.rows()] =
                        y.data()[source as usize + col * y.rows()];
                }
            }
        }
    }
    // The second output is the common time base, NOT the input shift vector.
    Ok(vec![
        value(y.rows(), cols, data)?,
        value(y.rows(), 1, (1..=y.rows()).map(|i| i as f64).collect())?,
    ])
}
