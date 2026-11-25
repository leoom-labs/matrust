//! fit(x, y, 'smoothingspline'): the cubic smoothing spline of csaps on the
//! (possibly normalized) included data, with the fit's weights.
use super::super::poly_interp::{CubicPp, smoothing_data, smoothing_spline};
use super::model::Knots;
use crate::Result;

/// A fitted smoothing spline: the smoothing parameter used and its pieces.
#[derive(Clone)]
pub(in crate::engine) struct Smoothed {
    pub p: f64,
    pub pp: CubicPp,
}

/// Knots (sorted distinct sites, weighted-mean values) carrying the spline;
/// `p` None picks the default smoothing parameter.
pub(super) fn knots(x: &[f64], y: &[f64], w: &[f64], p: Option<f64>) -> Result<Knots> {
    let data = smoothing_data("fit", x, &[y.to_vec()], w)?;
    let (pp, p) = smoothing_spline("fit", &data, p)?;
    let [values] = <[Vec<f64>; 1]>::try_from(data.y).expect("one component");
    Ok(Knots {
        x: data.x,
        y: values,
        smooth: Some(Box::new(Smoothed { p, pp })),
    })
}

pub(super) fn values(s: &Smoothed, x: &[f64]) -> Vec<f64> {
    x.iter().map(|&v| s.pp.eval(v, 0)).collect()
}

/// Display lines after the formula.
pub(super) fn describe(s: &Smoothed) -> String {
    format!("     Smoothing parameter:\n       p = {}\n", g8(s.p))
}

/// Eight significant digits without trailing zeros (%.8g for 0 <= v <= 1).
fn g8(v: f64) -> String {
    if v == 0. || v == 1. || !v.is_finite() {
        return format!("{v}");
    }
    let exp = v.abs().log10().floor() as i32;
    if exp < -4 {
        let s = format!("{v:.7e}");
        let (m, e) = s.split_once('e').unwrap();
        let m = m.trim_end_matches('0').trim_end_matches('.');
        let e: i32 = e.parse().unwrap();
        return format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    let decimals = (7 - exp).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn eight_digits() {
        assert_eq!(super::g8(0.99999904123), "0.99999904");
        assert_eq!(super::g8(0.3), "0.3");
        assert_eq!(super::g8(1.), "1");
        assert_eq!(super::g8(1.2345678912e-6), "1.2345679e-06");
    }
}
