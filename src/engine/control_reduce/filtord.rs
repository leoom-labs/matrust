//! filtord: order of a digital filter, the degree in z^-1 of its rational
//! transfer function B(z)/A(z) (no pole/zero cancellation). Trailing zero
//! coefficients add no delay and are ignored; leading zeros are delays and
//! count. An SOS matrix is K-by-6 [b0 b1 b2 a0 a1 a2]; its order is
//! max(sum of numerator degrees, sum of denominator degrees), the degrees of
//! the products of the sections.
use super::super::{Engine, Execution, Value};
use crate::{Matrix, Result};

/// Degree in z^-1: index of the last nonzero coefficient (0 if none).
fn degree(nonzero: impl Iterator<Item = bool>) -> usize {
    nonzero
        .enumerate()
        .filter(|&(_, nz)| nz)
        .map(|(i, _)| i)
        .last()
        .unwrap_or(0)
}

fn coefficients(v: &Value, what: &str) -> Result<(usize, usize, Vec<bool>)> {
    let (rows, cols, nonzero): (usize, usize, Vec<bool>) = match v {
        Value::Matrix(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|&x| x != 0.).collect(),
        ),
        Value::Complex(m) => (
            m.rows(),
            m.cols(),
            m.data().iter().map(|z| z.re != 0. || z.im != 0.).collect(),
        ),
        _ => return Err(format!("filtord: {what} must be numeric")),
    };
    let finite = match v {
        Value::Matrix(m) => m.data().iter().all(|x| x.is_finite()),
        Value::Complex(m) => m
            .data()
            .iter()
            .all(|z| z.re.is_finite() && z.im.is_finite()),
        _ => false,
    };
    if !finite {
        return Err(format!("filtord: {what} must be finite"));
    }
    Ok((rows, cols, nonzero))
}

fn is_sos(v: &Value) -> bool {
    match v {
        Value::Matrix(m) if m.cols() == 6 && m.rows() >= 1 => {
            // A single 1-by-6 row is an SOS section only when it is
            // normalized (a0 = 1); otherwise it is an FIR numerator.
            m.rows() > 1 || m.data()[3] == 1.
        }
        _ => false,
    }
}

pub(super) fn filtord(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    if args.is_empty() {
        return Err("filtord: not enough input arguments".into());
    }
    if args.len() > 2 {
        return Err("filtord: too many input arguments".into());
    }
    if !matches!(args[0], Value::Matrix(_) | Value::Complex(_)) {
        if args.len() == 1
            && let Some(v) = e.obj_call("filtord", args, out)?
        {
            return Ok(v);
        }
        return Err(
            "filtord: input must be coefficient vectors b and a, an SOS matrix or a \
             digitalFilter"
                .into(),
        );
    }
    let order = if args.len() == 1 && is_sos(&args[0]) {
        let (rows, _, nz) = coefficients(&args[0], "SOS")?;
        let at = |r: usize, c: usize| nz[c * rows + r];
        let mut num = 0;
        let mut den = 0;
        for r in 0..rows {
            if !at(r, 3) {
                return Err("filtord: every SOS section needs a nonzero a0 (column 4)".into());
            }
            num += degree((0..3).map(|c| at(r, c)));
            den += degree((3..6).map(|c| at(r, c)));
        }
        num.max(den)
    } else {
        let mut degrees = [0usize; 2];
        for (k, v) in args.iter().enumerate() {
            let what = if k == 0 { "b" } else { "a" };
            let (rows, cols, nz) = coefficients(v, what)?;
            if rows > 1 && cols > 1 {
                return Err(format!(
                    "filtord: {what} must be a vector (an SOS matrix must be K-by-6)"
                ));
            }
            if k == 1 && !nz.first().copied().unwrap_or(false) {
                return Err("filtord: the first denominator coefficient must be nonzero".into());
            }
            degrees[k] = degree(nz.into_iter());
        }
        degrees[0].max(degrees[1])
    };
    Ok(Value::Matrix(Matrix::scalar(order as f64)))
}
