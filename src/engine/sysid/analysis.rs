//! iddata construction, indexing and detrending; model constructors and
//! conversion to the engine's LTI values; sim, predict, compare and resid.
//! compare: fit = 100 (1 - |y - yhat| / |y - mean(y)|) per output channel,
//! initial state by least squares over the free responses of the sampled
//! realisation ('e', default) or zero ('z'). resid: correlations
//! R(tau, i, j) = 1/N sum_t z_i(t) z_j(t + tau) of z = [e u] (MathWorks resid
//! page); S.CrossCorr(tau) = R_eu(tau) = 1/N sum_t e(t + tau) u(t) for
//! tau = -M..M (Ljung's residual-input convention: R(tau, 2, 1) for tau >= 0,
//! R(-tau, 1, 2) for tau < 0); 99 % bounds
//! 2.5758/sqrt(N) for the normalised autocorrelation and 2.5758 sqrt(P/N),
//! P = sum_{|k|<=M} rho_e(k) rho_u(k), for the cross-correlation (Ljung,
//! System Identification, 2nd ed., sec. 16.6, eqs. 16.64-16.71).
use super::super::{Engine, Execution, Value, control_lti};
use super::lti::{Model, Proc, Structure, trim};
use super::obj::{
    IdData, IdModel, MAX_ORDER, MAX_SAMPLES, chars, is_data, is_model, make_struct, num, real_of,
    scalar, scalar_of, text_of, value_class,
};
use crate::syntax::Expr;
use crate::{Array, CellArray, Matrix, Result};
use nalgebra::{DMatrix, DVector};
use std::sync::Arc;

const Z995: f64 = 2.5758293035489004;
/// Polynomial degree limit of the engine's tf values (control_lti MAX_DEGREE).
const LTI_DEGREE: usize = 256;
/// Inner-loop visits allowed for one resid call (a few seconds).
const RESID_WORK: u64 = 10_000_000_000;

fn as_column(m: Matrix) -> Result<Matrix> {
    if m.rows() == 1 && m.cols() > 1 {
        Ok(m.transpose())
    } else {
        Ok(m)
    }
}
fn finite(m: &Matrix, who: &str) -> Result<()> {
    if m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "{who}: data must be finite (missing samples are not supported yet)"
        ));
    }
    Ok(())
}
fn names_value(e: &Engine, v: &Value, n: usize, what: &str) -> Result<Arc<Array>> {
    if let Some(t) = text_of(v) {
        if n != 1 {
            return Err(format!(
                "iddata: {what} needs one name per channel (a cell array)"
            ));
        }
        let slot = chars(e, &t)?;
        return Ok(Arc::new(
            CellArray::from_shared(1, 1, vec![slot], &e.cancel)?.into(),
        ));
    }
    if let Value::Container(Array::Cell(c)) = v
        && c.len() == n
        && (0..n).all(|i| {
            c.get_linear(i)
                .is_ok_and(|x| text_of(&Value::from(x.as_ref().clone())).is_some())
        })
    {
        let slots = (0..n)
            .map(|i| c.get_linear(i).cloned())
            .collect::<Result<Vec<_>>>()?;
        return Ok(Arc::new(
            CellArray::from_shared(n, 1, slots, &e.cancel)?.into(),
        ));
    }
    Err(format!(
        "iddata: {what} must be text or a cell array of {n} names"
    ))
}

/// iddata(y, u, Ts, Name, Value, ...)
pub(super) fn iddata(e: &Engine, args: &[Value]) -> Result<Value> {
    let named = |v: &Value| text_of(v).is_some_and(|t| !t.is_empty());
    let positional = args.iter().take(3).take_while(|v| !named(v)).count();
    if positional == 0 {
        return Err("iddata: expected output data y as the first argument".into());
    }
    let y = as_column(real_of(&args[0], "iddata: y")?)?;
    let u = match args.get(1).filter(|_| positional >= 2) {
        Some(v) if text_of(v).is_some() => Matrix::from_column_vec(y.rows(), 0, Vec::new())?,
        Some(v) => {
            let m = real_of(v, "iddata: u")?;
            if m.is_empty() {
                Matrix::from_column_vec(y.rows(), 0, Vec::new())?
            } else {
                as_column(m)?
            }
        }
        None => Matrix::from_column_vec(y.rows(), 0, Vec::new())?,
    };
    if y.is_empty() {
        return Err("iddata: y must not be empty".into());
    }
    if y.rows() > MAX_SAMPLES || y.cols() > 64 || u.cols() > 64 {
        return Err("iddata: the data exceeds the size bound (1e7 samples, 64 channels)".into());
    }
    if u.rows() != y.rows() {
        return Err(format!(
            "iddata: the output ({} samples) and input ({} samples) must have the same number of samples",
            y.rows(),
            u.rows()
        ));
    }
    finite(&y, "iddata")?;
    finite(&u, "iddata")?;
    let mut ts = 1.;
    if positional >= 3 {
        ts = scalar_of(&args[2], "iddata: Ts")?;
    }
    let mut d = IdData {
        ts,
        tstart: 0.,
        meta: IdData::defaults(e, y.cols(), u.cols())?,
        y,
        u,
    };
    let opts = super::est::options(
        &args[positional..],
        "iddata",
        &[
            "Ts",
            "Tstart",
            "OutputName",
            "InputName",
            "OutputUnit",
            "InputUnit",
            "TimeUnit",
            "Name",
        ],
    )?;
    for (n, v) in opts {
        match n.as_str() {
            "Ts" => d.ts = scalar_of(&v, "iddata: Ts")?,
            "Tstart" => d.tstart = scalar_of(&v, "iddata: Tstart")?,
            "OutputName" | "OutputUnit" => {
                let a = names_value(e, &v, d.ny(), &n)?;
                set(&mut d.meta, &n, a);
            }
            "InputName" | "InputUnit" => {
                let a = names_value(e, &v, d.nu(), &n)?;
                set(&mut d.meta, &n, a);
            }
            _ => {
                let t = text_of(&v).ok_or_else(|| format!("iddata: {n} must be text"))?;
                set(&mut d.meta, &n, chars(e, &t)?);
            }
        }
    }
    if (d.ts.is_nan() || d.ts <= 0.) || !d.ts.is_finite() {
        return Err("iddata: Ts must be a positive finite sample time".into());
    }
    if !d.tstart.is_finite() {
        return Err("iddata: Tstart must be finite".into());
    }
    d.encode(e)
}
fn set(meta: &mut Vec<(String, Arc<Array>)>, n: &str, a: Arc<Array>) {
    if let Some(s) = meta.iter_mut().find(|(k, _)| k == n) {
        s.1 = a;
    } else {
        meta.push((n.into(), a));
    }
}
pub(super) fn data_of(v: &Value, who: &str) -> Result<IdData> {
    match v {
        Value::Container(a) if is_data(v) => IdData::decode(a),
        Value::Matrix(m) if m.cols() >= 1 && !m.is_nd() => {
            let n = m.rows();
            Ok(IdData {
                y: Matrix::from_column_slice(n, 1, &m.data()[..n])?,
                u: Matrix::from_column_slice(n, m.cols() - 1, &m.data()[n..])?,
                ts: 1.,
                tstart: 0.,
                meta: Vec::new(),
            })
        }
        _ => Err(format!(
            "{who}: expected an iddata object (or a numeric [y u] matrix)"
        )),
    }
}
pub(super) fn model_of(v: &Value, who: &str) -> Result<IdModel> {
    match v {
        Value::Container(a) if is_model(v) => IdModel::decode(a),
        _ => Err(format!(
            "{who}: expected an identified model (idpoly, idtf, idss or idproc)"
        )),
    }
}
fn select(e: &mut Engine, arg: &Expr, len: usize, out: &mut Execution) -> Result<Vec<usize>> {
    if matches!(arg, Expr::All) {
        return Ok((0..len).collect());
    }
    let v = e.eval(arg, Some(len), out)?;
    let idx: Vec<usize> = match &v {
        Value::Logical(l) => {
            if l.len() > len {
                return Err("Index exceeds the number of samples".into());
            }
            l.data()
                .iter()
                .enumerate()
                .filter(|(_, b)| **b != 0)
                .map(|(i, _)| i)
                .collect()
        }
        Value::Matrix(m) => m
            .data()
            .iter()
            .map(|&x| {
                if x >= 1. && x.fract() == 0. && x <= len as f64 {
                    Ok(x as usize - 1)
                } else if x >= 1. && x.fract() == 0. {
                    Err(format!("Index exceeds the number of samples ({len})"))
                } else {
                    Err("Array indices must be positive integers or logical values.".into())
                }
            })
            .collect::<Result<_>>()?,
        _ => return Err("iddata indices must be numeric or logical".into()),
    };
    Ok(idx)
}
fn rows_cols(m: &Matrix, rows: &[usize], cols: &[usize]) -> Result<Matrix> {
    let n = m.rows();
    let mut data = Vec::with_capacity(rows.len() * cols.len());
    for &c in cols {
        for &r in rows {
            data.push(m.data()[c * n + r]);
        }
    }
    Matrix::from_column_vec(rows.len(), cols.len(), data)
}
fn pick_names(e: &Engine, a: Option<&Arc<Array>>, cols: &[usize]) -> Result<Option<Arc<Array>>> {
    let Some(Array::Cell(c)) = a.map(|x| x.as_ref()) else {
        return Ok(None);
    };
    let slots = cols
        .iter()
        .map(|&i| c.get_linear(i).cloned())
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(Arc::new(
        CellArray::from_shared(cols.len(), 1, slots, &e.cancel)?.into(),
    )))
}
/// data(samples), data(samples, outputs), data(samples, outputs, inputs).
pub(super) fn index(
    e: &mut Engine,
    a: &Array,
    args: &[Expr],
    out: &mut Execution,
) -> Result<Value> {
    let mut d = IdData::decode(a)?;
    if args.is_empty() || args.len() > 3 {
        return Err("iddata indexing is data(samples, outputs, inputs)".into());
    }
    let rows = select(e, &args[0], d.samples(), out)?;
    let ycols = match args.get(1) {
        Some(x) => select(e, x, d.ny(), out)?,
        None => (0..d.ny()).collect(),
    };
    let ucols = match args.get(2) {
        Some(x) => select(e, x, d.nu(), out)?,
        None => (0..d.nu()).collect(),
    };
    if rows.is_empty() || ycols.is_empty() {
        return Err("iddata indexing must keep at least one sample and one output".into());
    }
    let first = rows[0];
    let elements = ycols
        .len()
        .checked_add(ucols.len())
        .and_then(|c| c.checked_mul(rows.len()))
        .ok_or("iddata indexing: the result exceeds the element limit")?;
    if rows.len() > MAX_SAMPLES {
        return Err(format!(
            "iddata indexing: the result exceeds {MAX_SAMPLES} samples"
        ));
    }
    e.preflight_elements(elements)?;
    let y = rows_cols(&d.y, &rows, &ycols)?;
    let u = rows_cols(&d.u, &rows, &ucols)?;
    for (n, cols) in [
        ("OutputName", &ycols),
        ("OutputUnit", &ycols),
        ("InputName", &ucols),
        ("InputUnit", &ucols),
    ] {
        if let Some(x) = pick_names(e, d.meta(n), cols)? {
            set(&mut d.meta, n, x);
        }
    }
    d.tstart += first as f64 * d.ts;
    d.y = y;
    d.u = u;
    d.encode(e)
}
fn detrend_cols(m: &Matrix, linear: bool) -> Result<Matrix> {
    let n = m.rows();
    let mut data = m.data().to_vec();
    if n == 0 {
        return Ok(m.clone());
    }
    let nf = n as f64;
    let tm = (nf - 1.) / 2.;
    let stt: f64 = (0..n).map(|t| (t as f64 - tm) * (t as f64 - tm)).sum();
    for c in 0..m.cols() {
        let col = &mut data[c * n..(c + 1) * n];
        let mean = col.iter().sum::<f64>() / nf;
        let slope = if linear && stt > 0. {
            col.iter()
                .enumerate()
                .map(|(t, v)| (t as f64 - tm) * (v - mean))
                .sum::<f64>()
                / stt
        } else {
            0.
        };
        for (t, v) in col.iter_mut().enumerate() {
            *v -= mean + slope * (t as f64 - tm);
        }
    }
    Matrix::from_column_vec(n, m.cols(), data)
}
/// detrend(data) removes means; detrend(data, 1) removes least-squares lines.
pub(super) fn detrend(e: &Engine, args: &[Value]) -> Result<Value> {
    let mut d = data_of(&args[0], "detrend")?;
    let linear = match args.get(1) {
        None => false,
        Some(v) => match scalar_of(v, "detrend: type")? {
            0. => false,
            1. => true,
            _ => return Err("detrend: type must be 0 (mean) or 1 (linear) for iddata".into()),
        },
    };
    if args.len() > 2 {
        return Err("detrend: too many input arguments for iddata".into());
    }
    d.y = detrend_cols(&d.y, linear)?;
    d.u = detrend_cols(&d.u, linear)?;
    d.encode(e)
}

// ------------------------------------------------------- model creation --
fn vec_arg(v: Option<&Value>, who: &str, what: &str, default: &[f64]) -> Result<Vec<f64>> {
    let Some(v) = v else {
        return Ok(default.to_vec());
    };
    let m = real_of(v, &format!("{who}: {what}"))?;
    if m.is_empty() {
        return Ok(default.to_vec());
    }
    if m.rows() > 1 && m.cols() > 1 {
        return Err(format!("{who}: {what} must be a vector"));
    }
    if m.len() > 2 * MAX_ORDER + 1 || m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "{who}: {what} must be a finite vector of bounded length"
        ));
    }
    Ok(m.data().to_vec())
}
/// idpoly, idtf, idss, idproc constructors.
pub(super) fn construct(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    if let Some(Value::Container(array)) = args.first()
        && let Some(lti) = control_lti::decode(e, array)?
    {
        if args.len() > 1 {
            return Err(format!("{name}: expected one LTI model"));
        }
        lti.require_delay_free(name)?;
        let ts = lti.ts;
        let st = if name == "idss" {
            let (a, b, c, d) = e.ctl_ss(&lti, out)?;
            let [a, b, c, d] =
                [a, b, c, d].map(|m| DMatrix::from_column_slice(m.rows(), m.cols(), m.data()));
            let k = DMatrix::zeros(a.nrows(), c.nrows());
            Structure::Ss { a, b, c, d, k }
        } else {
            lti.require_siso(name)?;
            let (num, den) = e.ctl_tf(&lti, out)?;
            let numv = num.data().to_vec();
            let denv = den.data().to_vec();
            if name == "idpoly" {
                if ts == 0. {
                    return Err(
                        "idpoly: continuous-time polynomial models are not supported yet".into(),
                    );
                }
                // descending z powers -> q^-1 arrays of one length.
                let len = numv.len().max(denv.len());
                let mut b = vec![0.; len - numv.len()];
                b.extend(&numv);
                let mut a = denv.clone();
                a.resize(len, 0.);
                let lead = a[0];
                let a: Vec<f64> = a.iter().map(|x| x / lead).collect();
                let b: Vec<f64> = b.iter().map(|x| x / lead).collect();
                let a = trim_trailing(a);
                Structure::Poly {
                    a,
                    b,
                    c: vec![1.],
                    d: vec![1.],
                    f: vec![1.],
                }
            } else {
                Structure::Tf {
                    num: numv,
                    den: denv,
                }
            }
        };
        let m = IdModel::new(
            e,
            Model {
                s: st,
                ts,
                delay: 0.,
                noise_variance: 0.,
            },
            0.,
            0.,
        )?;
        return m.encode(e);
    }
    let positional = args.iter().take_while(|v| text_of(v).is_none()).count();
    let pos = &args[..positional];
    let opts = if name == "idproc" {
        Vec::new()
    } else {
        let allowed: &[&str] = if name == "idss" {
            &[
                "Ts",
                "InputDelay",
                "NoiseVariance",
                "InputName",
                "OutputName",
            ]
        } else {
            &[
                "Ts",
                "IODelay",
                "InputDelay",
                "Variable",
                "NoiseVariance",
                "InputName",
                "OutputName",
            ]
        };
        super::est::options(&args[positional..], name, allowed)?
    };
    let ts_opt = opts
        .iter()
        .find(|(n, _)| n == "Ts")
        .map(|(_, v)| scalar_of(v, &format!("{name}: Ts")))
        .transpose()?;
    let (st, ts, nv) = match name {
        "idpoly" => {
            if pos.is_empty() || pos.len() > 7 {
                return Err("idpoly: use idpoly(A, B, C, D, F, NoiseVariance, Ts)".into());
            }
            let a = vec_arg(pos.first(), name, "A", &[1.])?;
            let b = vec_arg(pos.get(1), name, "B", &[])?;
            let c = vec_arg(pos.get(2), name, "C", &[1.])?;
            let d = vec_arg(pos.get(3), name, "D", &[1.])?;
            let f = vec_arg(pos.get(4), name, "F", &[1.])?;
            for (n, p) in [("A", &a), ("C", &c), ("D", &d), ("F", &f)] {
                if p[0] != 1. {
                    return Err(format!("idpoly: {n} must be monic (leading coefficient 1)"));
                }
            }
            let nv = match pos.get(5) {
                Some(v) => scalar_of(v, "idpoly: NoiseVariance")?,
                None => 1.,
            };
            let ts = match (ts_opt, pos.get(6)) {
                (Some(t), _) => t,
                (None, Some(v)) => scalar_of(v, "idpoly: Ts")?,
                (None, None) => 1.,
            };
            if ts.is_nan() || ts <= 0. {
                return Err("idpoly: Ts must be positive (continuous-time polynomial models are not supported yet)".into());
            }
            (Structure::Poly { a, b, c, d, f }, ts, nv)
        }
        "idtf" => {
            if pos.len() < 2 || pos.len() > 3 {
                return Err("idtf: use idtf(num, den) or idtf(num, den, Ts)".into());
            }
            let num = vec_arg(pos.first(), name, "num", &[0.])?;
            let den = vec_arg(pos.get(1), name, "den", &[])?;
            if den.iter().all(|x| *x == 0.) {
                return Err("idtf: the denominator cannot be zero".into());
            }
            if trim(&num).len() > trim(&den).len() {
                return Err(
                    "idtf: improper models (more zeros than poles) are not supported".into(),
                );
            }
            let ts = match (ts_opt, pos.get(2)) {
                (Some(t), _) => t,
                (None, Some(v)) => scalar_of(v, "idtf: Ts")?,
                (None, None) => 0.,
            };
            (Structure::Tf { num, den }, ts, 0.)
        }
        "idss" => {
            if pos.len() < 4 || pos.len() > 7 {
                return Err("idss: use idss(A, B, C, D, K, x0, Ts)".into());
            }
            let mat = |i: usize, what: &str| -> Result<DMatrix<f64>> {
                let m = real_of(&pos[i], &format!("idss: {what}"))?;
                if m.data().iter().any(|x| !x.is_finite()) {
                    return Err(format!("idss: {what} must be finite"));
                }
                Ok(DMatrix::from_column_slice(m.rows(), m.cols(), m.data()))
            };
            let (a, b, c, mut d) = (mat(0, "A")?, mat(1, "B")?, mat(2, "C")?, mat(3, "D")?);
            let n = a.nrows();
            if a.ncols() != n || n > 256 || b.nrows() != n || c.ncols() != n {
                return Err("idss: A, B, C, D and K have inconsistent sizes".into());
            }
            // D (ny x nu) and the default K (nx x ny) are built here: admit first.
            let (ny, nu) = (c.nrows(), b.ncols());
            let elements = ny
                .checked_mul(nu)
                .and_then(|x| x.checked_add(n.checked_mul(ny)?))
                .ok_or("idss: the model exceeds the element limit")?;
            e.preflight_elements(elements)?;
            if d.is_empty() {
                d = DMatrix::zeros(ny, nu);
            }
            let k = if pos.len() > 4 && !real_of(&pos[4], "idss: K")?.is_empty() {
                mat(4, "K")?
            } else {
                DMatrix::zeros(a.nrows(), c.nrows())
            };
            let ts = match (ts_opt, pos.get(6)) {
                (Some(t), _) => t,
                (None, Some(v)) => scalar_of(v, "idss: Ts")?,
                (None, None) => 0.,
            };
            (Structure::Ss { a, b, c, d, k }, ts, 0.)
        }
        _ => {
            if positional != 0 {
                return Err("idproc: use idproc(Type, 'Kp', value, ...)".into());
            }
            let t = args
                .first()
                .and_then(text_of)
                .ok_or("idproc: expected the model type, e.g. 'P1D'")?;
            let mut p = Proc::parse(&t)?;
            for n in p.names() {
                p.set(n, f64::NAN);
            }
            let rest = &args[1..];
            let allowed = ["Kp", "Tp1", "Tp2", "Tp3", "Tw", "Zeta", "Td", "Tz"];
            for (n, v) in super::est::options(rest, "idproc", &allowed)? {
                if !p.names().contains(&n.as_str()) {
                    return Err(format!(
                        "idproc: {n} is not a parameter of a {} model",
                        p.type_name()
                    ));
                }
                p.set(&n, scalar_of(&v, &format!("idproc: {n}"))?);
            }
            (Structure::Proc(p), 0., 0.)
        }
    };
    if (ts.is_nan() || ts < 0.) || !ts.is_finite() {
        return Err(format!("{name}: Ts must be 0 or a positive sample time"));
    }
    let (mut st, mut nv) = (st, nv);
    let (mut io_delay, mut input_delay) = (0., 0.);
    let mut names = Vec::new();
    for (n, v) in &opts {
        match n.as_str() {
            "NoiseVariance" => nv = scalar_of(v, &format!("{name}: NoiseVariance"))?,
            "IODelay" => io_delay = scalar_of(v, &format!("{name}: IODelay"))?,
            "InputDelay" => input_delay = scalar_of(v, &format!("{name}: InputDelay"))?,
            "Variable" => {
                let t = text_of(v).ok_or_else(|| format!("{name}: Variable must be text"))?;
                st = variable(name, &t, ts, st)?;
            }
            "InputName" | "OutputName" => names.push((n.clone(), v.clone())),
            _ => {}
        }
    }
    let mut m = IdModel::new(
        e,
        Model {
            s: st,
            ts,
            delay: 0.,
            noise_variance: nv,
        },
        io_delay,
        input_delay,
    )?;
    for (n, v) in names {
        let count = match (&m.model.s, n.as_str()) {
            (Structure::Ss { b, .. }, "InputName") => b.ncols(),
            (Structure::Ss { c, .. }, _) => c.nrows(),
            (Structure::Poly { b, .. }, "InputName") if b.is_empty() => 0,
            _ => 1,
        };
        let cell = names_value(e, &v, count, &n).map_err(|x| x.replacen("iddata", name, 1))?;
        m.set_meta(&n, cell);
    }
    // Validate through the same decoder every later call uses.
    let v = m.encode(e)?;
    if let Value::Container(a) = &v {
        IdModel::decode(a)?;
    }
    Ok(v)
}
/// The 'Variable' option: idpoly is always in z^-1 (q^-1); idtf takes s/p
/// (continuous), z/q (descending powers) or z^-1/q^-1 (ascending powers of
/// the delay operator, converted to descending powers of z).
fn variable(name: &str, t: &str, ts: f64, st: Structure) -> Result<Structure> {
    let v = t.trim();
    let bad = |what: &str| -> Result<Structure> { Err(format!("{name}: Variable '{v}' {what}")) };
    match (name, v) {
        ("idpoly", "z^-1" | "q^-1") => Ok(st),
        ("idpoly", _) => bad("is not supported; use 'z^-1' or 'q^-1'"),
        ("idtf", "s" | "p") if ts == 0. => Ok(st),
        ("idtf", "z" | "q") if ts > 0. => Ok(st),
        ("idtf", "z^-1" | "q^-1") if ts > 0. => match st {
            Structure::Tf { num, den } => {
                let len = num.len().max(den.len());
                let (mut num, mut den) = (num, den);
                num.resize(len, 0.);
                den.resize(len, 0.);
                if trim(&den).is_empty() || den[0] == 0. {
                    return bad("needs a nonzero leading denominator coefficient");
                }
                Ok(Structure::Tf { num, den })
            }
            other => Ok(other),
        },
        ("idtf", "s" | "p" | "z" | "q" | "z^-1" | "q^-1") => bad("does not match the sample time"),
        _ => bad("is not supported; use 's', 'p', 'z', 'q', 'z^-1' or 'q^-1'"),
    }
}
fn trim_trailing(mut a: Vec<f64>) -> Vec<f64> {
    while a.len() > 1 && *a.last().unwrap() == 0. {
        a.pop();
    }
    a
}
fn proc_ready(m: &IdModel, who: &str) -> Result<()> {
    if let Structure::Proc(p) = &m.model.s
        && p.names().iter().any(|n| !p.get(n).is_finite())
    {
        return Err(format!(
            "{who}: the process model has unset (NaN) parameters"
        ));
    }
    Ok(())
}
/// tf or ss value of the measured dynamics of an identified model.
pub(super) fn to_lti(e: &mut Engine, m: &IdModel, out: &mut Execution) -> Result<Value> {
    proc_ready(m, "LTI conversion")?;
    let model = &m.model;
    let delay = model.delay
        + match &model.s {
            Structure::Proc(p) if p.delay => p.td,
            _ => 0.,
        };
    if model.continuous() && delay > 0. {
        return Err(format!(
            "LTI conversion of a {} model with a continuous input/output delay of {} s is unsupported (use sim, compare or predict)",
            m.class(),
            super::obj::g(delay)
        ));
    }
    let shift = if model.continuous() {
        0
    } else {
        model.delay_samples()
    };
    let ts = Value::Matrix(Matrix::scalar(model.ts));
    let rowv = |v: &[f64]| -> Result<Value> {
        Ok(Value::Matrix(Matrix::from_column_slice(1, v.len(), v)?))
    };
    if let Structure::Ss { a, b, c, d, .. } = &model.s {
        if shift > 0 {
            return Err(
                "ss: discrete state-space models with an input delay cannot be converted yet"
                    .into(),
            );
        }
        let mv = |x: &DMatrix<f64>| -> Result<Value> {
            Ok(Value::Matrix(Matrix::from_column_slice(
                x.nrows(),
                x.ncols(),
                x.as_slice(),
            )?))
        };
        return e.call("ss", vec![mv(a)?, mv(b)?, mv(c)?, mv(d)?, ts], out);
    }
    let (num, mut den) = model.rational().expect("rational");
    if !model.continuous() && shift > 0 {
        // z^-shift = 1/z^shift: `shift` trailing zeros on the denominator
        // only (num and den are descending powers of z).
        let len = den
            .len()
            .checked_add(shift)
            .filter(|l| *l <= LTI_DEGREE + 1)
            .ok_or_else(|| {
                format!(
                    "tf: the delay of {shift} samples exceeds the LTI degree limit ({LTI_DEGREE})"
                )
            })?;
        den.resize(len, 0.);
    }
    let mut num = trim(&num);
    if num.is_empty() {
        num.push(0.);
    }
    // Identification conversions have always returned a monic transfer
    // function even when the estimated process denominator had another scale.
    let lead = den[0];
    for coefficient in num.iter_mut().chain(&mut den) {
        *coefficient /= lead;
    }
    e.call("tf", vec![rowv(&num)?, rowv(&den)?, ts], out)
}
pub(super) fn lti_args(
    e: &mut Engine,
    values: Vec<Value>,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if !values.iter().any(is_model) {
        return Ok(values);
    }
    values
        .into_iter()
        .map(|v| {
            if is_model(&v) {
                let m = model_of(&v, "LTI conversion")?;
                to_lti(e, &m, out)
            } else {
                Ok(v)
            }
        })
        .collect()
}

// ------------------------------------------------------------- analysis --
fn output_data(e: &Engine, d: &IdData, yhat: Vec<f64>) -> Result<Value> {
    let mut r = d.clone();
    r.y = Matrix::from_column_vec(yhat.len(), 1, yhat)?;
    if r.meta.is_empty() {
        r.meta = IdData::defaults(e, 1, r.nu())?;
    }
    r.encode(e)
}
/// y = sim(sys, data | u)
pub(super) fn sim(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err("sim: use y = sim(sys, data) or y = sim(sys, u)".into());
    }
    if args.len() > 2 {
        return Err("sim: options are not supported yet".into());
    }
    let m = model_of(&args[0], "sim")?;
    proc_ready(&m, "sim")?;
    if is_data(&args[1]) {
        let d = data_of(&args[1], "sim")?;
        if d.nu() != 1 {
            return Err("sim: the data must have exactly one input channel".into());
        }
        let u = IdData::col(&d.u, 0);
        let y = m.model.simulate(&u, d.ts)?;
        return output_data(e, &d, y);
    }
    let u = real_of(&args[1], "sim: u")?;
    if u.cols() != 1 && u.rows() != 1 {
        return Err("sim: u must be a vector (one input channel)".into());
    }
    if m.model.continuous() {
        return Err(
            "sim: a continuous-time model needs an iddata input (for the sample time)".into(),
        );
    }
    let y = m.model.simulate(u.data(), m.model.ts)?;
    Ok(Value::Matrix(Matrix::from_column_vec(y.len(), 1, y)?))
}
fn horizon(v: Option<&Value>, who: &str, default: usize) -> Result<usize> {
    let Some(v) = v else { return Ok(default) };
    let k = scalar_of(v, &format!("{who}: prediction horizon"))?;
    if k == f64::INFINITY {
        return Ok(usize::MAX);
    }
    if !(k >= 1. && k.fract() == 0.) {
        return Err(format!(
            "{who}: the prediction horizon must be a positive integer or Inf"
        ));
    }
    Ok(k.min(1e15) as usize)
}
/// yp = predict(sys, data, k)
pub(super) fn predict(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err("predict: use yp = predict(sys, data, k)".into());
    }
    let (mv, dv) = if is_model(&args[0]) {
        (&args[0], &args[1])
    } else {
        (&args[1], &args[0])
    };
    let m = model_of(mv, "predict")?;
    proc_ready(&m, "predict")?;
    let d = data_of(dv, "predict")?;
    let (y, u) = d.siso("predict", false)?;
    let k = horizon(args.get(2), "predict", 1)?;
    let yp = m.model.predict(&y, &u, d.ts, k, &|| e.check_cancel())?;
    output_data(e, &d, yp)
}
fn fit_percent(y: &[f64], yh: &[f64]) -> f64 {
    let n = y.len() as f64;
    let mean = y.iter().sum::<f64>() / n;
    let num = y
        .iter()
        .zip(yh)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        .sqrt();
    let den = y
        .iter()
        .map(|a| (a - mean) * (a - mean))
        .sum::<f64>()
        .sqrt();
    100. * (1. - num / den)
}
/// [y, fit, x0] = compare(data, sys, k, 'InitialCondition', ic)
pub(super) fn compare(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() < 2 {
        return Err("compare: use [y, fit, x0] = compare(data, sys)".into());
    }
    let (dv, mv) = if is_data(&args[0]) || !is_model(&args[0]) {
        (&args[0], &args[1])
    } else {
        (&args[1], &args[0])
    };
    if args.iter().skip(2).any(is_model) {
        return Err("compare: one model per call is supported yet".into());
    }
    let d = data_of(dv, "compare")?;
    let m = model_of(mv, "compare")?;
    proc_ready(&m, "compare")?;
    let (y, u) = d.siso("compare", false)?;
    let mut rest = &args[2..];
    let mut k = usize::MAX;
    if let Some(v) = rest.first()
        && text_of(v).is_none()
    {
        k = horizon(Some(v), "compare", usize::MAX)?;
        rest = &rest[1..];
    }
    let opts = super::est::options(rest, "compare", &["InitialCondition"])?;
    let mut ic = Value::Text("e".into());
    for (_, v) in opts {
        ic = v;
    }
    let dss = m.model.dss(d.ts)?;
    let n = dss.order();
    if n.saturating_mul(y.len()) > super::est::WORK_LIMIT {
        return Err("compare: samples times model states exceeds the work bound".into());
    }
    let mut x0 = DVector::zeros(n);
    let yhat = if k < y.len() {
        m.model.predict(&y, &u, d.ts, k, &|| e.check_cancel())?
    } else {
        match text_of(&ic).map(|s| s.to_ascii_lowercase()) {
            Some(s) if s == "z" || s == "zero" => dss.simulate(&u, None),
            Some(s) if s == "e" || s == "estimate" || s == "auto" => {
                let base = dss.simulate(&u, None);
                if n > 0 {
                    let free = dss.free(y.len());
                    let r: Vec<f64> = y.iter().zip(&base).map(|(a, b)| a - b).collect();
                    x0 = super::num::lstsq(&free, &DVector::from_vec(r))?;
                }
                dss.simulate(&u, Some(&x0))
            }
            Some(s) => {
                return Err(format!(
                    "compare: unknown InitialCondition '{s}'; use 'e', 'z' or a state vector"
                ));
            }
            None => {
                let v = real_of(&ic, "compare: InitialCondition")?;
                if v.len() != n {
                    return Err(format!("compare: the initial state must have {n} elements"));
                }
                x0 = DVector::from_column_slice(v.data());
                dss.simulate(&u, Some(&x0))
            }
        }
    };
    let fit = fit_percent(&y, &yhat);
    Ok(vec![
        output_data(e, &d, yhat)?,
        Value::Matrix(Matrix::scalar(fit)),
        Value::Matrix(Matrix::from_column_slice(n, 1, x0.as_slice())?),
    ])
}
fn cov(a: &[f64], b: &[f64], lag: isize) -> f64 {
    // 1/N sum_t a(t + lag) b(t)
    let n = a.len() as isize;
    let mut s = 0.;
    for t in 0..n {
        let i = t + lag;
        if i >= 0 && i < n {
            s += a[i as usize] * b[t as usize];
        }
    }
    s / n as f64
}
/// [E, R, S] = resid(data, sys, M)
pub(super) fn resid(e: &Engine, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() < 2 {
        return Err("resid: use [E, R] = resid(data, sys)".into());
    }
    let (dv, mv) = if is_data(&args[0]) || !is_model(&args[0]) {
        (&args[0], &args[1])
    } else {
        (&args[1], &args[0])
    };
    let d = data_of(dv, "resid")?;
    let m = model_of(mv, "resid")?;
    proc_ready(&m, "resid")?;
    let (y, u) = d.siso("resid", false)?;
    let mut lags = 25usize;
    let mut rest = &args[2..];
    if let Some(t) = rest.first().and_then(text_of)
        && t.eq_ignore_ascii_case("corr")
    {
        rest = &rest[1..];
    }
    if let Some(v) = rest.first()
        && text_of(v).is_none()
    {
        let x = scalar_of(v, "resid: M")?;
        if !(x >= 1. && x.fract() == 0. && x <= 10_000.) {
            return Err("resid: the number of lags M must be a positive integer".into());
        }
        lags = x as usize;
        rest = &rest[1..];
    }
    for (_, v) in super::est::options(rest, "resid", &["MaxLag"])? {
        let x = scalar_of(&v, "resid: MaxLag")?;
        if !(x >= 1. && x.fract() == 0. && x <= 10_000.) {
            return Err("resid: MaxLag must be a positive integer".into());
        }
        lags = x as usize;
    }
    let n = y.len();
    lags = lags.min(n.saturating_sub(1)).max(1);
    let has_u = d.nu() == 1;
    let k = if has_u { 2u64 } else { 1 };
    // R, the autocorrelation and the two-sided cross-correlation: each lag
    // visits all N samples once.
    let work = (n as u64)
        .saturating_mul(lags as u64 + 1)
        .saturating_mul(k * k + 4);
    if work > RESID_WORK {
        return Err(format!(
            "resid: samples times lags exceeds the work bound ({RESID_WORK}); use fewer lags"
        ));
    }
    let eps = m.model.innovations(&y, &u, d.ts)?;
    let z: Vec<&[f64]> = if has_u { vec![&eps, &u] } else { vec![&eps] };
    let k = z.len();
    let mut r = vec![0.; (lags + 1) * k * k];
    for j in 0..k {
        for i in 0..k {
            for tau in 0..=lags {
                e.check_cancel()?;
                // R(tau, i, j) = 1/N sum_t z_i(t) z_j(t + tau).
                r[tau + (lags + 1) * (i + k * j)] = cov(z[j], z[i], tau as isize);
            }
        }
    }
    let rm = Matrix::from_column_vec(lags + 1, k * k, r)?.with_dims(&[lags + 1, k, k])?;
    let re0 = cov(&eps, &eps, 0);
    let auto: Vec<f64> = (0..=lags)
        .map(|t| {
            e.check_cancel()?;
            Ok(cov(&eps, &eps, t as isize) / re0)
        })
        .collect::<Result<_>>()?;
    let auto_bound = Z995 / (n as f64).sqrt();
    let mut fields = vec![
        (
            "Lags",
            num(Matrix::from_column_vec(
                lags + 1,
                1,
                (0..=lags).map(|x| x as f64).collect(),
            )?),
        ),
        (
            "AutoCorr",
            num(Matrix::from_column_vec(lags + 1, 1, auto.clone())?),
        ),
        ("AutoBound", scalar(auto_bound)),
    ];
    let lag_axis: Vec<f64> = (-(lags as isize)..=lags as isize)
        .map(|x| x as f64)
        .collect();
    if has_u {
        let ru0 = cov(&u, &u, 0);
        let norm = (re0 * ru0).sqrt();
        let cross: Vec<f64> = (-(lags as isize)..=lags as isize)
            .map(|t| {
                e.check_cancel()?;
                Ok(cov(&eps, &u, t) / norm)
            })
            .collect::<Result<_>>()?;
        let mut pu = 0.;
        for (t, a) in auto.iter().enumerate().take(lags + 1).skip(1) {
            e.check_cancel()?;
            pu += a * cov(&u, &u, t as isize) / ru0;
        }
        let p = 1. + 2. * pu;
        fields.push((
            "CrossLags",
            num(Matrix::from_column_vec(lag_axis.len(), 1, lag_axis)?),
        ));
        fields.push((
            "CrossCorr",
            num(Matrix::from_column_vec(cross.len(), 1, cross)?),
        ));
        fields.push(("CrossBound", scalar(Z995 * (p.max(0.) / n as f64).sqrt())));
    } else {
        fields.push(("CrossLags", num(Matrix::from_column_vec(0, 1, Vec::new())?)));
        fields.push(("CrossCorr", num(Matrix::from_column_vec(0, 1, Vec::new())?)));
        fields.push(("CrossBound", scalar(f64::NAN)));
    }
    fields.push(("Confidence", scalar(99.)));
    let s = make_struct(e, fields)?;
    Ok(vec![
        output_data(e, &d, eps)?,
        Value::Matrix(rm),
        Value::Container(s),
    ])
}
/// size(data) = [N ny nu]; size of a model = [ny nu].
pub(super) fn size(e: &Engine, args: &[Value]) -> Result<Value> {
    let dims: Vec<f64> = if is_data(&args[0]) {
        let d = data_of(&args[0], "size")?;
        vec![d.samples() as f64, d.ny() as f64, d.nu() as f64]
    } else {
        let m = model_of(&args[0], "size")?;
        match &m.model.s {
            Structure::Ss { b, c, .. } => vec![c.nrows() as f64, b.ncols() as f64],
            Structure::Poly { b, .. } if b.is_empty() => vec![1., 0.],
            _ => vec![1., 1.],
        }
    };
    let _ = e;
    match args.get(1) {
        None => Ok(Value::Matrix(Matrix::from_column_slice(
            1,
            dims.len(),
            &dims,
        )?)),
        Some(v) => {
            let k = scalar_of(v, "size: dim")?;
            if !(k >= 1. && k.fract() == 0.) {
                return Err("size: dimension must be a positive integer".into());
            }
            Ok(Value::Matrix(Matrix::scalar(
                dims.get(k as usize - 1).copied().unwrap_or(1.),
            )))
        }
    }
}
pub(super) fn class_name(v: &Value) -> Option<&'static str> {
    value_class(v)
}
