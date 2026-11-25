//! plot(cfit, ...) and plot(sfit, ...) through the ordinary plot builtins:
//! data as markers, the fit as a line over the data range, dashed prediction
//! bounds, residuals with a zero line; surfaces as surf plus plot3 markers.
use super::super::{Engine, Execution, Value};
use super::Fitted;
use super::fit::real_matrix;
use super::options::text_of;
use crate::{Matrix, Result};

const CURVE_POINTS: usize = 400;
const SURFACE_POINTS: usize = 41;
const TYPES: [&str; 8] = [
    "fit",
    "predfunc",
    "predobs",
    "residuals",
    "stresiduals",
    "deriv1",
    "deriv2",
    "integral",
];

fn row(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}
fn text(t: &str) -> Value {
    Value::Text(t.into())
}

/// The x range to draw: the data given, else the fit's data, else the axes.
fn x_range(
    e: &mut Engine,
    f: &Fitted,
    data: Option<&[f64]>,
    out: &mut Execution,
) -> Result<(f64, f64)> {
    let fold = |v: &[f64]| {
        v.iter()
            .filter(|x| x.is_finite())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
                (a.min(x), b.max(x))
            })
    };
    let (mut lo, mut hi) = match data {
        Some(x) if !x.is_empty() => fold(x),
        _ => f.range.first().copied().unwrap_or((f64::NAN, f64::NAN)),
    };
    if !(lo.is_finite() && hi.is_finite()) {
        let lim = e.call("xlim", Vec::new(), out)?;
        let m = real_matrix(e, &lim, "plot", "axis limits")?;
        (lo, hi) = (m.data()[0], m.data()[1]);
    }
    if lo == hi {
        (lo, hi) = (lo - 1., hi + 1.);
    }
    Ok((lo, hi))
}

pub(super) fn plot(
    e: &mut Engine,
    f: &Fitted,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if f.ft.surface() {
        return surface(e, f, rest, nargout, out);
    }
    let mut fit_spec = "r-".to_owned();
    let mut data_spec = "b.".to_owned();
    let mut data: Option<(Vec<f64>, Vec<f64>)> = None;
    let mut types: Vec<String> = Vec::new();
    let mut level = 0.95;
    let mut k = 0;
    let is_type =
        |v: &Value| text_of(v).is_some_and(|t| TYPES.contains(&t.to_ascii_lowercase().as_str()));
    if let Some(t) = rest.first().and_then(text_of)
        && !is_type(&rest[0])
    {
        fit_spec = t;
        k = 1;
    }
    if rest.len() >= k + 2
        && text_of(&rest[k]).is_none()
        && text_of(&rest[k + 1]).is_none()
        && !matches!(rest[k], Value::Container(crate::Array::Cell(_)))
    {
        let x = real_matrix(e, &rest[k], "plot", "x data")?;
        let y = real_matrix(e, &rest[k + 1], "plot", "y data")?;
        if x.len() != y.len() {
            return Err("plot: x and y data must have the same number of elements".into());
        }
        data = Some((x.data().to_vec(), y.data().to_vec()));
        k += 2;
        if let Some(t) = rest.get(k).and_then(text_of)
            && !is_type(&rest[k])
        {
            data_spec = t;
            k += 1;
        }
    }
    while k < rest.len() {
        match &rest[k] {
            v if is_type(v) => types.push(text_of(v).unwrap().to_ascii_lowercase()),
            Value::Container(crate::Array::Cell(_)) => {
                for t in super::model::name_list(&rest[k], "plot types")? {
                    let t = t.to_ascii_lowercase();
                    if !TYPES.contains(&t.as_str()) {
                        return Err(format!("plot: unknown plot type '{t}'"));
                    }
                    types.push(t);
                }
            }
            v if text_of(v).is_none() => {
                let m = real_matrix(e, v, "plot", "the confidence level")?;
                if m.len() != 1 || !(m.data()[0] > 0. && m.data()[0] < 1.) {
                    return Err("plot: the confidence level must be between 0 and 1".into());
                }
                level = m.data()[0];
            }
            v => {
                return Err(format!(
                    "plot: unknown plot type '{}'; use fit, predfunc, predobs, residuals, deriv1, deriv2 or integral",
                    text_of(v).unwrap_or_default()
                ));
            }
        }
        k += 1;
    }
    if types.is_empty() {
        types.push("fit".into());
    }
    let mut handles = Vec::new();
    for (i, kind) in types.iter().enumerate() {
        if types.len() > 1 {
            let args = [types.len() as f64, 1., (i + 1) as f64]
                .map(|v| Value::Matrix(Matrix::scalar(v)))
                .to_vec();
            e.call("subplot", args, out)?;
        }
        let h = one_plot(e, f, kind, data.as_ref(), &fit_spec, &data_spec, level, out)?;
        handles.push(h);
    }
    if nargout == 0 {
        return Ok(Vec::new());
    }
    if handles.len() == 1 {
        return Ok(handles);
    }
    Ok(vec![e.call("vertcat", handles, out)?])
}

#[allow(clippy::too_many_arguments)]
fn one_plot(
    e: &mut Engine,
    f: &Fitted,
    kind: &str,
    data: Option<&(Vec<f64>, Vec<f64>)>,
    fit_spec: &str,
    data_spec: &str,
    level: f64,
    out: &mut Execution,
) -> Result<Value> {
    let (lo, hi) = x_range(e, f, data.map(|d| d.0.as_slice()), out)?;
    let xf: Vec<f64> = (0..CURVE_POINTS)
        .map(|i| lo + (hi - lo) * i as f64 / (CURVE_POINTS - 1) as f64)
        .collect();
    let mut args = Vec::new();
    let mut labels: Vec<&str> = Vec::new();
    match kind {
        "residuals" | "stresiduals" => {
            let Some((x, y)) = data else {
                return Err(
                    "plot: residual plots need the data: plot(fitobject, x, y, 'residuals')".into(),
                );
            };
            let fx = super::eval_fitted(e, f, std::slice::from_ref(x), out)?;
            let mut r: Vec<f64> = y.iter().zip(&fx).map(|(y, f)| y - f).collect();
            if kind == "stresiduals" {
                let s = f.mse.sqrt();
                if s.is_nan() || s <= 0. {
                    return Err(
                        "plot: standardized residuals need a fit with error degrees of freedom"
                            .into(),
                    );
                }
                r.iter_mut().for_each(|v| *v /= s);
            }
            args.extend([row(x.clone())?, row(r)?, text(data_spec)]);
            args.extend([row(vec![lo, hi])?, row(vec![0., 0.])?, text("k-")]);
            labels.extend(["data", "zero line"]);
        }
        "deriv1" | "deriv2" => {
            let v = super::calculus::differentiate(e, f, &[row(xf.clone())?], 2, out)?;
            let d = if kind == "deriv1" { &v[0] } else { &v[1] };
            args.extend([row(xf)?, d.clone(), text(fit_spec)]);
            labels.push(if kind == "deriv1" {
                "first derivative"
            } else {
                "second derivative"
            });
        }
        "integral" => {
            let x0 = Value::Matrix(Matrix::scalar(lo));
            let v = super::calculus::integrate(e, f, &[row(xf.clone())?, x0], out)?;
            args.extend([row(xf)?, v, text(fit_spec)]);
            labels.push("integral");
        }
        _ => {
            if let Some((x, y)) = data {
                args.extend([row(x.clone())?, row(y.clone())?, text(data_spec)]);
                labels.push("data");
            }
            let yf = super::eval_fitted(e, f, std::slice::from_ref(&xf), out)?;
            args.extend([row(xf.clone())?, row(yf)?, text(fit_spec)]);
            labels.push("fitted curve");
            if kind == "predfunc" || kind == "predobs" {
                let intopt = if kind == "predobs" {
                    "observation"
                } else {
                    "functional"
                };
                let rest = [
                    row(xf.clone())?,
                    Value::Matrix(Matrix::scalar(level)),
                    text(intopt),
                ];
                let v = super::inference::predint(e, f, &rest, 1, out)?;
                let ci = real_matrix(e, &v[0], "plot", "bounds")?;
                let n = xf.len();
                let style = format!(
                    "{}--",
                    fit_spec
                        .chars()
                        .find(|c| "rgbcmykw".contains(*c))
                        .unwrap_or('r')
                );
                // Both bounds as one line broken by NaN: one legend entry.
                let mut bx = xf.clone();
                bx.push(f64::NAN);
                bx.extend_from_slice(&xf);
                let mut by = ci.data()[..n].to_vec();
                by.push(f64::NAN);
                by.extend_from_slice(&ci.data()[n..]);
                args.extend([row(bx)?, row(by)?, text(&style)]);
                labels.push("prediction bounds");
            }
        }
    }
    let h = e.call("plot", args, out)?;
    e.call("legend", labels.into_iter().map(text).collect(), out)?;
    Ok(h)
}

/// plot(sfit), plot(sfit, [x y], z): the fitted surface over the data range,
/// with the data as markers.
fn surface(
    e: &mut Engine,
    f: &Fitted,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let mut data = None;
    match rest {
        [] => {}
        [xy, z] | [xy, z, _] if text_of(xy).is_none() => {
            let xy = real_matrix(e, xy, "plot", "[x y] data")?;
            let z = real_matrix(e, z, "plot", "z data")?;
            if xy.cols() != 2 || xy.rows() != z.len() {
                return Err(
                    "plot: surface data must be plot(sfit, [x y], z) with one z per row".into(),
                );
            }
            let n = xy.rows();
            data = Some((
                xy.data()[..n].to_vec(),
                xy.data()[n..].to_vec(),
                z.data().to_vec(),
            ));
        }
        _ => return Err("plot: expected plot(sfit) or plot(sfit, [x y], z)".into()),
    }
    let range = |k: usize, v: Option<&Vec<f64>>| {
        let (lo, hi) = match v {
            Some(v) => v
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
                    (a.min(x), b.max(x))
                }),
            None => f.range.get(k).copied().unwrap_or((0., 1.)),
        };
        if lo.is_finite() && hi.is_finite() && lo < hi {
            (lo, hi)
        } else {
            (0., 1.)
        }
    };
    let (xr, yr) = (
        range(0, data.as_ref().map(|d| &d.0)),
        range(1, data.as_ref().map(|d| &d.1)),
    );
    let s = SURFACE_POINTS;
    let mut gx = Vec::with_capacity(s * s);
    let mut gy = Vec::with_capacity(s * s);
    for c in 0..s {
        for r in 0..s {
            gx.push(xr.0 + (xr.1 - xr.0) * c as f64 / (s - 1) as f64);
            gy.push(yr.0 + (yr.1 - yr.0) * r as f64 / (s - 1) as f64);
        }
    }
    let gz = super::eval_fitted(e, f, &[gx.clone(), gy.clone()], out)?;
    let grid =
        |v: Vec<f64>| -> Result<Value> { Ok(Value::Matrix(Matrix::from_column_vec(s, s, v)?)) };
    let mut handles = vec![e.call("surf", vec![grid(gx)?, grid(gy)?, grid(gz)?], out)?];
    if let Some((x, y, z)) = data {
        let held = e.figure.as_ref().is_some_and(|fig| fig.hold);
        if !held {
            e.call("hold", vec![text("on")], out)?;
        }
        let h = e.call("plot3", vec![row(x)?, row(y)?, row(z)?, text("ko")], out);
        if !held {
            e.call("hold", vec![text("off")], out)?;
        }
        handles.push(h?);
    }
    if nargout == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![e.call("vertcat", handles, out)?])
}
